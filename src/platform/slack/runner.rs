//! Acknowledgment and heartbeat handling never wait for enrichment or model work.
use super::{events, SlackAdapter};
use crate::{
    command::{self, ActionTarget, Actor, AdminService},
    platform::{
        outgoing::ConnectionStatus,
        registry::RoomPolicy,
        runtime::{EventSender, ShutdownReceiver},
        types::*,
        Adapter, Platform, PlatformError, PlatformResult,
    },
    Bot, BotError,
};
use futures_util::{Sink, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_tungstenite::tungstenite::Message;

struct Seen {
    set: HashSet<String>,
    order: VecDeque<String>,
}
impl Seen {
    fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }
    fn accept(&mut self, id: &str) -> bool {
        if !self.set.insert(id.into()) {
            return false;
        }
        self.order.push_back(id.into());
        if self.order.len() > 4096 {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

async fn acknowledge<S>(sink: &mut S, envelope: &Value) -> PlatformResult<()>
where
    S: Sink<Message> + Unpin,
{
    if let Some(id) = envelope["envelope_id"].as_str() {
        sink.send(Message::Text(json!({"envelope_id":id}).to_string().into()))
            .await
            .map_err(|_| PlatformError::Disconnected)?;
    }
    Ok(())
}

fn command_args(
    text: &str,
    bot_id: &str,
) -> Result<Option<command::ActionArgs>, command::ActionError> {
    let body = if let Some(rest) = text.strip_prefix('!') {
        rest
    } else if let Some(rest) = text.strip_prefix(&format!("<@{bot_id}>")) {
        if !rest.starts_with(char::is_whitespace) {
            return Ok(None);
        }
        if command::schema::lookup(rest.split_whitespace().next().unwrap_or("")).is_none() {
            return Ok(None);
        }
        rest.trim()
    } else {
        return Ok(None);
    };
    // The shared explicit-prefix parser intentionally requires a whitespace boundary.
    command::parser::parse_text(
        &format!("slack-command {body}"),
        &command::parser::CommandPrefix::Explicit("slack-command".into()),
    )
}

async fn socket_url(adapter: &SlackAdapter, token: &str) -> PlatformResult<String> {
    let response = adapter
        .client
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| {
            PlatformError::Transport("Slack apps.connections.open request failed".into())
        })?;
    if response.status().as_u16() == 429 {
        return Err(PlatformError::RateLimited {
            retry_after: response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse().ok())
                .map(Duration::from_secs),
        });
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| PlatformError::Transport("Slack socket response is invalid JSON".into()))?;
    if body["ok"].as_bool() != Some(true) {
        let error = body["error"].as_str().unwrap_or("unknown_error");
        // Do not echo arbitrary server-controlled fields, URLs or credentials.
        return Err(PlatformError::Transport(format!(
            "Slack apps.connections.open: {}",
            match error {
                "invalid_auth" => "invalid_auth",
                "not_authed" => "not_authed",
                "token_revoked" => "token_revoked",
                "missing_scope" => "missing_scope",
                "ratelimited" => "ratelimited",
                _ => "API failure",
            }
        )));
    }
    let url = body["url"]
        .as_str()
        .ok_or_else(|| PlatformError::Transport("Slack socket response omitted URL".into()))?;
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| PlatformError::Transport("Slack socket URL is invalid".into()))?;
    if parsed.scheme() != "wss" || !parsed.host_str().is_some_and(|h| h.ends_with(".slack.com")) {
        return Err(PlatformError::Transport(
            "Slack socket URL is not a Slack secure endpoint".into(),
        ));
    }
    Ok(url.into())
}

fn lifecycle(
    connection: &ConnectionId,
    team: &str,
    event: ChatEvent,
    sequence: u64,
) -> PlatformResult<EventEnvelope> {
    Ok(EventEnvelope {
        connection: connection.clone(),
        event_id: EventKey::new(
            PlatformKind::Slack,
            &[team, &format!("lifecycle-{sequence}")],
        )?,
        occurred_at_unix_ms: crate::now_unix().saturating_mul(1000),
        event,
    })
}

async fn dispatch(
    bot: &Arc<Bot>,
    adapter: &Arc<Adapter>,
    ingress: &EventSender,
    payload: Value,
    seen: &mut Seen,
    commands: &mut JoinSet<()>,
) -> PlatformResult<()> {
    let Adapter::Slack(slack) = adapter.as_ref() else {
        return Err(PlatformError::InvalidIdentity(KeyError));
    };
    if payload["team_id"].as_str() != Some(slack.team.as_str()) {
        return Err(PlatformError::Transport(
            "Slack app and bot tokens must belong to the same workspace".into(),
        ));
    }
    let raw = &payload["event"];
    let Some(mut event) = events::decode(&slack.connection, &slack.team, &slack.account.id, raw)?
    else {
        return Ok(());
    };
    let native_event = payload["event_id"].as_str().unwrap_or("");
    if native_event.is_empty() {
        return Err(PlatformError::Transport(
            "Slack event omitted event_id".into(),
        ));
    }
    let id = match &event {
        ChatEvent::MessageCreated(message) => message.id.to_string(),
        _ => native_event.to_string(),
    };
    if !seen.accept(&id) {
        return Ok(());
    }
    let room = match &event {
        ChatEvent::MessageCreated(m) => &m.room,
        ChatEvent::MemberJoined { room, .. } => room,
        ChatEvent::ReactionAdded(r) => &r.room,
        _ => return Ok(()),
    };
    if !bot
        .adapters
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allows(&slack.connection, room)
    {
        return Ok(());
    }
    let known = slack.rooms.lock().contains_key(&room.id);
    if !known {
        let channel = room.id.components().nth(1).ok_or(KeyError)??;
        let info = slack
            .api("conversations.info", json!({"channel":channel}))
            .await?;
        if info["channel"]["is_member"].as_bool() != Some(true) {
            return Ok(());
        }
        let mut known_room = room.clone();
        known_room.kind = if info["channel"]["is_private"].as_bool() == Some(true) {
            RoomKind::Group
        } else {
            RoomKind::Channel
        };
        known_room.name = info["channel"]["name"].as_str().map(str::to_owned);
        slack.rooms.lock().insert(room.id.clone(), known_room);
    }
    let room = room.clone();
    bot.adapters
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .bind_discovered(&slack.connection, room.clone())?;
    match &mut event {
        ChatEvent::MessageCreated(message) => {
            message.author = slack.resolve_user(&message.author.id).await?;
            if message.author.automation != Automation::Human {
                return Ok(());
            }
            for mention in &message.mentions {
                if let Ok(user) = slack.resolve_user(mention).await {
                    let native = super::transport::native_user(&slack.team, mention)?;
                    message.text = message
                        .text
                        .replace(&format!("@{native}"), &format!("@{}", user.display_name));
                }
            }
            let bot_id = super::transport::native_user(&slack.team, &slack.account.id)?;
            let parsed = command_args(raw["text"].as_str().unwrap_or(""), &bot_id);
            if !matches!(parsed, Ok(None)) {
                if commands.len() >= 32 {
                    return Err(PlatformError::Transport(
                        "Slack command queue is full".into(),
                    ));
                }
                let bot = bot.clone();
                let adapter = adapter.clone();
                let user = message.author.clone();
                let room = message.room.clone();
                commands.spawn(async move {
                    let Adapter::Slack(slack) = adapter.as_ref() else { return; };
                    let target = ActionTarget { connection:Some(slack.connection.clone()),room:Some(room.clone()) };
                    let response = match parsed {
                        Ok(Some(args)) => match AdminService::new(&bot).execute(&Actor::User(user.id.clone()), &target,args).await { Ok(result)=>command::render::plain(&result,true),Err(error)=>Err(error) },
                        Err(error)=>Err(error), Ok(None)=>return,
                    };
                    let text = response.unwrap_or_else(|e|command::render::localized_description(&e));
                    let channel = room.id.components().nth(1).ok_or(KeyError).and_then(std::convert::identity);
                    let native_user = super::transport::native_user(&slack.team,&user.id);
                    if let (Ok(channel),Ok(native_user)) = (channel,native_user) {
                        for part in super::render::split(&text) {
                            let mut body=json!({"channel":channel,"user":native_user,"text":super::render::escape(&part),"parse":"none","unfurl_links":false,"unfurl_media":false});
                            if let Some(thread)=&room.thread { if let Some(Ok(ts))=thread.components().nth(2) { body["thread_ts"]=json!(ts); } }
                            if let Err(error)=slack.api("chat.postEphemeral",body).await { log::warn!("Slack private command response: {error}");break; }
                        }
                    }
                });
                return Ok(());
            }
        }
        ChatEvent::MemberJoined { user, .. } => {
            *user = slack.resolve_user(&user.id).await?;
        }
        ChatEvent::ReactionAdded(reaction) => {
            reaction.actor = slack.resolve_user(&reaction.actor.id).await?;
            let channel = reaction.target.id.components().nth(1).ok_or(KeyError)??;
            let ts = reaction.target.id.components().nth(2).ok_or(KeyError)??;
            slack.reserve_history().await;
            let data = slack
                .api(
                    "conversations.history",
                    json!({"channel":channel,"latest":ts,"oldest":ts,"inclusive":true,"limit":1}),
                )
                .await?;
            if let Some(message) = data["messages"]
                .as_array()
                .and_then(|messages| messages.first())
            {
                reaction.target.author = message["user"]
                    .as_str()
                    .map(|u| UserKey::new(PlatformKind::Slack, &[&slack.team, u]))
                    .transpose()?;
                reaction.original_text = message["text"].as_str().map(str::to_owned);
            }
        }
        _ => {}
    }
    ingress
        .send(EventEnvelope {
            connection: slack.connection.clone(),
            event_id: EventKey::new(PlatformKind::Slack, &[&slack.team, &id])?,
            occurred_at_unix_ms: raw["event_ts"]
                .as_str()
                .map(events::timestamp_ms)
                .unwrap_or_else(|| crate::now_unix().saturating_mul(1000)),
            event,
        })
        .await
}

type SlackSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_socket(
    slack: &SlackAdapter,
    app_token: &str,
    shutdown: &mut ShutdownReceiver,
) -> PlatformResult<Option<SlackSocket>> {
    let mut delay = Duration::from_secs(1);
    loop {
        if *shutdown.borrow() {
            return Ok(None);
        }
        let url = tokio::select! {
            value=socket_url(slack,app_token)=>match value {
                Ok(url)=>url,
                Err(error @ PlatformError::Transport(_)) if matches!(&error,PlatformError::Transport(message) if ["invalid_auth","not_authed","token_revoked","missing_scope"].iter().any(|native|message.ends_with(native)))=>return Err(error),
                Err(error)=>{
                    log::warn!("Slack socket URL unavailable; reconnecting: {error}");
                    let wait=match error {PlatformError::RateLimited{retry_after:Some(wait)}=>wait,_=>delay};
                    tokio::select!{_=tokio::time::sleep(wait)=>{},_=shutdown.changed()=>return Ok(None)};
                    delay=(delay*2).min(Duration::from_secs(30));continue;
                }
            },
            _=shutdown.changed()=>return Ok(None)
        };
        let socket = tokio::select! {value=tokio_tungstenite::connect_async(&url)=>value,_=shutdown.changed()=>return Ok(None)};
        match socket {
            Ok((socket, _)) => return Ok(Some(socket)),
            Err(_) => {
                log::warn!("Slack socket connection failed; reconnecting");
                tokio::select! {_=tokio::time::sleep(delay)=>{},_=shutdown.changed()=>return Ok(None)};
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

pub(crate) async fn run(
    bot: Arc<Bot>,
    connection: ConnectionId,
    app_token: String,
    bot_token: String,
    ingress: EventSender,
    mut shutdown: ShutdownReceiver,
) -> Result<(), BotError> {
    if *shutdown.borrow() {
        return Ok(());
    }
    let slack = SlackAdapter::authenticate(connection.clone(), bot_token).await?;
    log::info!("{}", crate::strings::t("slack.nickname.unsupported"));
    let approved = crate::config::runtime_value("SLACK_CHANNELS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|channel| RoomKey::new(PlatformKind::Slack, &[&slack.team, channel]))
        .collect::<Result<HashSet<_>, _>>()?;
    let admins = crate::owner_id()
        .filter(|key| {
            key.platform() == PlatformKind::Slack
                && key.components().next() == Some(Ok(slack.team.clone()))
        })
        .cloned()
        .into_iter()
        .collect();
    {
        let mut registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
        registry.set_room_policy(&connection, RoomPolicy::Approved(approved))?;
        registry.set_admin_users(&connection, admins)?;
    }
    let team = slack.team.clone();
    let rooms = slack.list_rooms().await?;
    let account = slack.account.clone();
    let Some(socket) = connect_socket(&slack, &app_token, &mut shutdown).await? else {
        return Ok(());
    };
    let mut initial_socket = Some(socket);
    let adapter = bot.register_adapter(Adapter::Slack(Box::new(slack)))?;
    let (sender, mut receiver) = mpsc::channel::<Value>(256);
    let worker_bot = bot.clone();
    let worker_adapter = adapter.clone();
    let worker_ingress = ingress.clone();
    let mut worker = tokio::spawn(async move {
        let mut seen = Seen::new();
        let mut commands = JoinSet::new();
        loop {
            tokio::select! { payload=receiver.recv()=>match payload {Some(payload)=>{if let Err(error)=dispatch(&worker_bot,&worker_adapter,&worker_ingress,payload,&mut seen,&mut commands).await {log::warn!("Slack event processing: {error}");}},None=>break}, result=commands.join_next(),if !commands.is_empty()=>{if let Some(Err(error))=result {log::error!("Slack command task: {error}");}} }
        }
    });
    let Adapter::Slack(slack) = adapter.as_ref() else {
        unreachable!("registered Slack adapter")
    };
    let mut sequence = 0;
    let mut started = false;
    let mut worker_completed = false;
    let result=async {
        loop {
            if *shutdown.borrow() { return Ok(()); }
            let mut socket=if let Some(socket)=initial_socket.take() {socket} else {
                tokio::select! {
                    value=connect_socket(slack,&app_token,&mut shutdown)=>match value? {Some(socket)=>socket,None=>return Ok(())},
                    result=&mut worker=>{worker_completed=true;let _=result;return Err(PlatformError::Transport("Slack event worker stopped".into()));},
                }
            };
            bot.adapters.lock().unwrap_or_else(|e|e.into_inner()).set_status(&connection,ConnectionStatus::Ready)?;
            sequence+=1;ingress.send(lifecycle(&connection,&team,ChatEvent::ConnectionReady(account.clone()),sequence)?).await?;
            if !started { sequence+=1;ingress.send(lifecycle(&connection,&team,ChatEvent::SpaceReady{space:SpaceRef{id:SpaceKey::new(PlatformKind::Slack,&[&team])?,name:None,preferred_room:rooms.first().map(|r|r.id.clone()),rooms:rooms.clone()},history:HistoryInitializationPolicy::AfterSuccess},sequence)?).await?;started=true; }
            loop {
                tokio::select! {
                    _=shutdown.changed()=>{let _=socket.close(None).await;return Ok(());},
                    result=&mut worker=>{worker_completed=true;let _=result;return Err(PlatformError::Transport("Slack event worker stopped".into()));},
                    message=socket.next()=>match message {
                        Some(Ok(Message::Text(text)))=>{
                            let mut envelope:Value=serde_json::from_str(&text).map_err(|_|PlatformError::Transport("Slack socket envelope is invalid JSON".into()))?;
                            if acknowledge(&mut socket,&envelope).await.is_err() { break; }
                            match envelope["type"].as_str() {
                                Some("disconnect")=>break,
                                Some("events_api")=>sender.try_send(envelope["payload"].take()).map_err(|_|PlatformError::Transport("Slack ingress queue is full or disconnected".into()))?,
                                _=>{}
                            }
                        },
                        Some(Ok(Message::Ping(bytes)))=>{if socket.send(Message::Pong(bytes)).await.is_err() {break;}},
                        Some(Ok(Message::Close(_)))|Some(Err(_))|None=>break,
                        _=>{}
                    }
                }
            }
            bot.adapters.lock().unwrap_or_else(|e|e.into_inner()).set_status(&connection,ConnectionStatus::Disconnected)?;
            tokio::select!{_=tokio::time::sleep(Duration::from_secs(1))=>{},_=shutdown.changed()=>return Ok(())};
        }
    }.await;
    if !worker_completed {
        worker.abort();
        let _ = worker.await;
    }
    sequence += 1;
    let _ = ingress
        .send(lifecycle(
            &connection,
            &team,
            ChatEvent::ConnectionClosed,
            sequence,
        )?)
        .await;
    result.map_err(BotError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedup_is_bounded() {
        let mut seen = Seen::new();
        assert!(seen.accept("same"));
        assert!(!seen.accept("same"));
        for index in 0..4096 {
            assert!(seen.accept(&index.to_string()));
        }
        assert_eq!(seen.set.len(), 4096);
        assert!(seen.accept("same"));
    }
    #[test]
    fn explicit_commands_and_bot_mentions_only() {
        assert!(matches!(
            command_args("!status", "U1").unwrap(),
            Some(command::ActionArgs::Status {})
        ));
        assert!(matches!(
            command_args("<@U1> status", "U1").unwrap(),
            Some(command::ActionArgs::Status {})
        ));
        assert!(command_args("status", "U1").unwrap().is_none());
        assert!(command_args("<@U2> status", "U1").unwrap().is_none());
        assert!(command_args("<@U1> hello there", "U1").unwrap().is_none());
        assert!(command_args("<@U1> ", "U1").unwrap().is_none());
        assert!(matches!(
            command_args("!unknown", "U1").unwrap(),
            Some(command::ActionArgs::Help {})
        ));
    }
    #[tokio::test]
    async fn ack_precedes_processing_even_for_redelivery() {
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let ack_log = log.clone();
        let sink = futures_util::sink::unfold(ack_log, |log, message: Message| async move {
            assert!(message.to_text().unwrap().contains("envelope-1"));
            log.lock().push("ack");
            Ok::<_, std::convert::Infallible>(log)
        });
        tokio::pin!(sink);
        let envelope = json!({"envelope_id":"envelope-1","type":"events_api"});
        acknowledge(&mut sink, &envelope).await.unwrap();
        log.lock().push("process");
        acknowledge(&mut sink, &envelope).await.unwrap();
        assert_eq!(*log.lock(), ["ack", "process", "ack"]);
    }

    #[tokio::test]
    #[ignore = "requires installed Slack app and real SLACK_APP_TOKEN / SLACK_BOT_TOKEN"]
    async fn live_socket_mode_connects_and_closes() {
        let app_token = std::env::var("SLACK_APP_TOKEN").expect("SLACK_APP_TOKEN required");
        let bot_token = std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN required");
        let adapter = SlackAdapter::authenticate(ConnectionId::new("slack-live-test"), bot_token)
            .await
            .expect("Slack bot authentication failed");
        let url = socket_url(&adapter, &app_token)
            .await
            .expect("Slack app authentication failed");
        let (mut socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|_| "Slack socket connection failed")
            .unwrap();
        let hello = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .expect("Slack hello timed out")
            .expect("Slack socket closed")
            .map_err(|_| "Slack socket receive failed")
            .unwrap();
        let body: Value = serde_json::from_str(hello.to_text().expect("Slack hello must be text"))
            .expect("Slack hello must be JSON");
        assert_eq!(body["type"], "hello");
        socket
            .close(None)
            .await
            .map_err(|_| "Slack socket close failed")
            .unwrap();
    }
}

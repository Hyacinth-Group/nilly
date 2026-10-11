//! Outbound long polling with a credential-free, account-scoped durable cursor.
use super::{
    client::TelegramClient,
    events::{self, Update, User},
    TelegramAdapter,
};
use crate::{
    command::{
        self,
        parser::{parse_text, CommandPrefix},
        ActionArgs, ActionTarget, Actor, AdminService,
    },
    memory,
    platform::{
        capabilities::MessageVisibility,
        registry::RoomPolicy,
        runtime::{EventSender, ShutdownReceiver},
        types::*,
        Adapter, Platform, PlatformError, PlatformResult,
    },
    Bot, BotError,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinSet;

pub(crate) fn parse_rooms(input: &str) -> PlatformResult<HashSet<RoomKey>> {
    if input.trim().is_empty() {
        return Ok(HashSet::new());
    }
    input
        .split(',')
        .map(str::trim)
        .map(|value| {
            let key = if value.starts_with("telegram:room:") {
                value.parse::<RoomKey>()?
            } else {
                let id: i64 = value
                    .parse()
                    .map_err(|_| PlatformError::InvalidIdentity(KeyError))?;
                if value != id.to_string() {
                    return Err(PlatformError::InvalidIdentity(KeyError));
                }
                events::room_key(id)
            };
            let id = super::chat_id(&key)?;
            if id == 0 || key != events::room_key(id) {
                return Err(PlatformError::InvalidIdentity(KeyError));
            }
            Ok(key)
        })
        .collect()
}
fn cursor_key(account: i64) -> String {
    format!("platform/telegram/{account}/offset")
}
fn load_offset(account: i64) -> Result<i64, BotError> {
    let value = memory::read_metadata(&cursor_key(account))?.unwrap_or(0);
    i64::try_from(value).map_err(|_| "invalid persisted Telegram update offset".into())
}
fn store_offset(account: i64, offset: i64) -> Result<(), BotError> {
    let offset = u64::try_from(offset).map_err(|_| "negative Telegram update offset")?;
    memory::write_metadata(&cursor_key(account), offset)
}
fn envelope(
    connection: &ConnectionId,
    id: &str,
    timestamp: i64,
    event: ChatEvent,
) -> EventEnvelope {
    EventEnvelope {
        connection: connection.clone(),
        event_id: EventKey::new(PlatformKind::Telegram, &[id]).expect("nonempty Telegram event id"),
        occurred_at_unix_ms: timestamp,
        event,
    }
}
async fn command_response(
    bot: Arc<Bot>,
    adapter: Arc<Adapter>,
    native: events::Message,
    parsed: Result<Option<ActionArgs>, command::ActionError>,
) {
    let Adapter::Telegram(telegram) = adapter.as_ref() else {
        return;
    };
    let Some(author) = native.from.as_ref() else {
        return;
    };
    let actor = Actor::User(events::user_key(author.id));
    let room = events::room_ref(&native);
    let approved = bot
        .adapters
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allows(telegram.connection_id(), &room);
    let target = ActionTarget {
        connection: Some(telegram.connection_id().clone()),
        room: approved.then_some(room.clone()),
    };
    let service = AdminService::new(&bot);
    // Authenticate even malformed commands before returning any private response.
    let authorization = service.authorize(&actor, &target, &ActionArgs::Status {});
    if let Err(error) = authorization {
        log::debug!("Telegram command denied: {error}");
        return;
    }
    let mut text = match parsed {
        Ok(Some(args)) => {
            // A successful getChat to this user's private chat proves the user has
            // initiated a DM; never execute mutations then publish private fallback.
            if telegram
                .client
                .call::<Value>("getChat", &json!({"chat_id":author.id}))
                .await
                .is_err()
            {
                log::warn!("{}", crate::strings::t("telegram.commands.private"));
                return;
            }
            let is_status = matches!(args, ActionArgs::Status {});
            match service.execute(&actor, &target, args).await {
                Ok(result) => {
                    let mut text = command::render::plain(&result, true)
                        .unwrap_or_else(|e| command::render::localized_description(&e));
                    if is_status {
                        let limited =
                            telegram.capabilities(&room).visibility != MessageVisibility::Ordinary;
                        if limited {
                            text.push('\n');
                            text.push_str(crate::strings::t("telegram.privacy_warning"));
                        }
                        text.push('\n');
                        text.push_str(crate::strings::t("telegram.nickname_warning"));
                        text.push('\n');
                        text.push_str(crate::strings::t("telegram.history_warning"));
                    }
                    text
                }
                Err(error) => command::render::localized_description(&error),
            }
        }
        Err(error) => command::render::localized_description(&error),
        Ok(None) => return,
    };
    if text.is_empty() {
        text = crate::strings::t("admin.error.private").to_owned();
    }
    if let Err(error) = telegram.private_text(author.id, &text).await {
        log::warn!("Telegram private command delivery: {error}");
    }
}

pub(crate) async fn run(
    bot: Arc<Bot>,
    connection: ConnectionId,
    token: String,
    rooms: HashSet<RoomKey>,
    owner: Option<UserKey>,
    ingress: EventSender,
    mut shutdown: ShutdownReceiver,
) -> Result<(), BotError> {
    let result = async {
        if *shutdown.borrow() { return Ok(()); }
        let client = Arc::new(TelegramClient::new(token)?);
        let get_me = json!({});
        let account: User = tokio::select! {
            result = client.call("getMe", &get_me) => result?,
            _ = shutdown.changed() => return Ok(()),
        };
        if account.can_read_all_group_messages != Some(true) { log::warn!("{}", crate::strings::t("telegram.privacy_warning")); }
        let mut offset = load_offset(account.id)?;
        {
            let mut registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
            registry.set_room_policy(&connection, RoomPolicy::Approved(rooms.clone()))?;
            registry.set_admin_users(&connection, owner.into_iter().collect())?;
        }
        let commands: Vec<_> = command::schema::definitions().iter().map(|definition| json!({"command":definition.id.as_str(),"description":definition.description().chars().take(256).collect::<String>()})).collect();
        client.call::<Value>("setMyCommands", &json!({"commands":commands})).await?;
        let adapter = bot.register_adapter(Adapter::Telegram(TelegramAdapter::new(connection.clone(), account.clone(), client.clone())))?;
        let Adapter::Telegram(telegram) = adapter.as_ref() else { unreachable!("registered Telegram adapter") };
        let mut refreshed = HashMap::new();
        for key in &rooms {
            let room = RoomRef::from_key(key.clone());
            match telegram.discover(&room).await {
                Ok(()) => {
                    bot.adapters.lock().unwrap_or_else(|e| e.into_inner()).bind_discovered(&connection, room)?;
                    refreshed.insert(key.clone(), Instant::now());
                }
                Err(error) => log::warn!("Telegram room [{key}]: {error}"),
            }
        }
        ingress.send(envelope(&connection, &format!("ready:{}", crate::now_unix()), crate::now_unix()*1000, ChatEvent::ConnectionReady(events::user_ref(&account)))).await?;
        let prefix = CommandPrefix::Telegram { bot_name: account.username.clone().unwrap_or_default() };
        let mut commands = JoinSet::new();
        loop {
            if *shutdown.borrow() { break; }
            let request = json!({"offset":offset,"timeout":30,"allowed_updates":["message","edited_message","my_chat_member"]});
            let updates: Vec<Update> = tokio::select! {
                result = client.call("getUpdates", &request) => match result {
                    Ok(updates) => updates,
                    Err(error @ PlatformError::PermissionDenied) => return Err(error.into()),
                    Err(error) => {
                        if matches!(&error, PlatformError::Transport(message) if message.contains("401")) { return Err(error.into()); }
                        log::warn!("Telegram polling: {error}");
                        let delay = match error { PlatformError::RateLimited { retry_after } => retry_after.unwrap_or(Duration::from_secs(5)), _ => Duration::from_secs(5) };
                        tokio::select! { _ = tokio::time::sleep(delay) => {}, _ = shutdown.changed() => break }
                        continue;
                    }
                },
                _ = shutdown.changed() => break,
            };
            for update in updates {
                if update.update_id < offset { continue; }
                if let Some(native) = &update.message {
                    let room = events::room_ref(native);
                    let approved = rooms.contains(&room.id);
                    if approved {
                        if refreshed.get(&room.id).is_none_or(|last| last.elapsed() >= Duration::from_secs(300)) {
                            telegram.discover(&room).await?;
                            refreshed.insert(room.id.clone(), Instant::now());
                        }
                        bot.adapters.lock().unwrap_or_else(|e| e.into_inner()).bind_discovered(&connection, room)?;
                    }
                    if native.sender_chat.is_none() && native.from.as_ref().is_some_and(|user| !user.is_bot && user.id != account.id) {
                        if let Some(text) = events::command_text(native) {
                            let parsed = if bot.stealth { Ok(None) } else { parse_text(text, &prefix) };
                            if !matches!(parsed, Ok(None)) {
                                while commands.len() >= 16 { let _ = commands.join_next().await; }
                                commands.spawn(command_response(bot.clone(), adapter.clone(), native.clone(), parsed));
                                offset = update.update_id.checked_add(1).ok_or("Telegram update offset overflow")?;
                                store_offset(account.id, offset)?;
                                continue;
                            }
                            // /start is Telegram's explicit private-chat opt-in, not
                            // personality observation or a shared administration action.
                            if native.chat.kind == "private" && text.split_whitespace().next() == Some("/start") {
                                offset = update.update_id.checked_add(1).ok_or("Telegram update offset overflow")?;
                                store_offset(account.id, offset)?;
                                continue;
                            }
                        }
                    }
                }
                for (index, event) in events::decode_update(&update, &connection, &account, bot.image_analysis).into_iter().enumerate() {
                    let timestamp = update.message.as_ref().map_or(crate::now_unix()*1000, |m| m.date.saturating_mul(1000));
                    ingress.send(envelope(&connection, &format!("{}:{index}", update.update_id), timestamp, event)).await?;
                }
                // Advance only after all events were accepted by bounded ingress.
                offset = update.update_id.checked_add(1).ok_or("Telegram update offset overflow")?;
                store_offset(account.id, offset)?;
                while commands.try_join_next().is_some() {}
            }
        }
        commands.abort_all();
        Ok(())
    }.await;
    let _ = ingress
        .send(envelope(
            &connection,
            &format!("closed:{}", crate::now_unix()),
            crate::now_unix() * 1000,
            ChatEvent::ConnectionClosed,
        ))
        .await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_config_is_qualified_signed_and_default_deny() {
        assert!(parse_rooms("").unwrap().is_empty());
        assert_eq!(
            parse_rooms("-100123, telegram:room:-77").unwrap(),
            HashSet::from([events::room_key(-100123), events::room_key(-77)])
        );
        for value in [
            "discord:room:7",
            "0",
            "telegram:room:abc",
            "telegram:room:-001",
        ] {
            assert!(parse_rooms(value).is_err());
        }
    }
    #[test]
    fn offset_persistence_round_trip_and_invalid_cursor_rejection() {
        crate::memory::test::ensure_test_db();
        let account = 8800000000000000077;
        store_offset(account, 781).unwrap();
        assert_eq!(load_offset(account).unwrap(), 781);
        store_offset(account, 782).unwrap();
        assert_eq!(load_offset(account).unwrap(), 782);
        assert!(store_offset(account, -1).is_err());
        memory::write_metadata(&cursor_key(account), u64::MAX).unwrap();
        assert!(load_offset(account).is_err());
        assert!(!cursor_key(account).contains(':'));
    }
}

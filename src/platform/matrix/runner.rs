use super::{events, render, MatrixAdapter, MatrixConfig};
use crate::{
    command::{self, ActionTarget, Actor, AdminService},
    platform::{
        outgoing::OutgoingMessage,
        registry::RoomPolicy,
        runtime::{EventSender, ShutdownReceiver},
        types::*,
        Adapter, Platform,
    },
    Bot, BotError,
};
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    config::{RequestConfig, SyncSettings},
    ruma::{
        api::client::{membership::get_member_events, room::create_room},
        RoomId, UserId,
    },
    Client, SessionMeta, SessionTokens,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;

#[derive(Serialize, Deserialize)]
struct SavedSession {
    homeserver: String,
    session: MatrixSession,
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), BotError> {
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(temp, path)?;
    Ok(())
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, BotError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn state_path(connection: &ConnectionId) -> PathBuf {
    Path::new(crate::STATE_DIR)
        .join("matrix")
        .join(encode_component(connection.as_str()))
}
pub(crate) async fn login(config: &MatrixConfig, path: &Path) -> Result<Client, BotError> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let client = Client::builder()
        .homeserver_url(&config.homeserver)
        .disable_well_known_lookup(true)
        .sqlite_store(path.join("store"), None)
        .request_config(
            RequestConfig::new()
                .timeout(Duration::from_secs(15))
                .retry_limit(1),
        )
        .build()
        .await?;
    let saved: Option<SavedSession> = load(&path.join("session.json"))?;
    if let Some(saved) = saved {
        if saved.homeserver != config.homeserver
            || saved.session.meta.user_id.as_str() != config.user
        {
            return Err("saved Matrix session belongs to a different homeserver/account; use a separate state directory".into());
        }
        // Token rotation is explicit and keeps the persisted device. whoami below
        // rejects a token for a different account/device rather than creating one.
        let mut session = saved.session;
        if let Some(token) = &config.access_token {
            session.tokens.access_token.clone_from(token);
        }
        client.restore_session(session).await?;
    } else if let Some(token) = &config.access_token {
        // Temporary metadata is only used for whoami; no sync/device/store data is
        // written until the server supplies the actual existing token device.
        let session = MatrixSession {
            meta: SessionMeta {
                user_id: UserId::parse(&config.user)?,
                device_id: "TOKEN-PREFLIGHT".into(),
            },
            tokens: SessionTokens {
                access_token: token.clone(),
                refresh_token: None,
            },
        };
        client.restore_session(session).await?;
        let identity = client.whoami().await?;
        if identity.user_id.as_str() != config.user {
            return Err("Matrix token does not belong to MATRIX_USER".into());
        }
        let device = identity
            .device_id
            .ok_or("Matrix token has no device; use a normal account/device token")?;
        // A new client is needed because SDK session metadata is immutable.
        let authenticated = Client::builder()
            .homeserver_url(&config.homeserver)
            .disable_well_known_lookup(true)
            .sqlite_store(path.join("store"), None)
            .request_config(
                RequestConfig::new()
                    .timeout(Duration::from_secs(15))
                    .retry_limit(1),
            )
            .build()
            .await?;
        authenticated
            .restore_session(MatrixSession {
                meta: SessionMeta {
                    user_id: identity.user_id,
                    device_id: device,
                },
                tokens: SessionTokens {
                    access_token: token.clone(),
                    refresh_token: None,
                },
            })
            .await?;
        persist_session(&authenticated, config, path)?;
        return Ok(authenticated);
    } else {
        client
            .matrix_auth()
            .login_username(
                &config.user,
                config
                    .password
                    .as_deref()
                    .ok_or("MATRIX_PASSWORD is missing")?,
            )
            .initial_device_display_name("Personality bot (plaintext)")
            .send()
            .await?;
    }
    let identity = client.whoami().await?;
    if identity.user_id.as_str() != config.user
        || identity.device_id.as_deref() != client.device_id()
    {
        return Err("Matrix session token account/device mismatch".into());
    }
    persist_session(&client, config, path)?;
    Ok(client)
}
fn persist_session(client: &Client, config: &MatrixConfig, path: &Path) -> Result<(), BotError> {
    save(
        &path.join("session.json"),
        &SavedSession {
            homeserver: config.homeserver.clone(),
            session: client
                .matrix_auth()
                .session()
                .ok_or("Matrix login did not produce a session")?,
        },
    )
}
fn auto_join(
    room: &RoomKey,
    inviter: Option<&UserKey>,
    owner: Option<&UserKey>,
    allowed: &HashSet<RoomKey>,
) -> bool {
    allowed.contains(room) || owner.is_some_and(|owner| inviter == Some(owner))
}
fn envelope(
    connection: &ConnectionId,
    id: &str,
    event: ChatEvent,
) -> Result<EventEnvelope, BotError> {
    Ok(EventEnvelope {
        connection: connection.clone(),
        event_id: EventKey::new(PlatformKind::Matrix, &[id])?,
        occurred_at_unix_ms: now(),
        event,
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn policy(
    bot: &Bot,
    connection: &ConnectionId,
    allowed: &HashSet<RoomKey>,
    owner: Option<&UserKey>,
) -> Result<(), BotError> {
    let mut registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
    registry.set_room_policy(connection, RoomPolicy::Approved(allowed.clone()))?;
    registry.set_admin_users(connection, owner.cloned().into_iter().collect())?;
    Ok(())
}
pub(super) fn command_text(text: &str) -> Option<String> {
    let rest = text.strip_prefix('!')?;
    if rest.starts_with(char::is_whitespace) || rest.is_empty() {
        return None;
    }
    if rest == "bot" || rest.starts_with("bot ") {
        Some(text.into())
    } else if command::schema::lookup(rest.split_whitespace().next().unwrap_or("")).is_some() {
        Some(format!("!bot {rest}"))
    } else {
        None
    }
}
async fn private_room(
    adapter: &MatrixAdapter,
    owner: &UserKey,
    path: &Path,
) -> Result<matrix_sdk::Room, BotError> {
    let native_owner = owner
        .components()
        .next()
        .ok_or("missing owner Matrix ID")??;
    let owner_id = UserId::parse(native_owner)?;
    let own = adapter.client.user_id().ok_or("Matrix not authenticated")?;
    let stored: Option<String> = load(&path.join("admin-room.json"))?;
    let room = if let Some(id) = stored {
        adapter
            .client
            .get_room(&RoomId::parse(id)?)
            .ok_or("private admin room unavailable; refusing public fallback")?
    } else {
        let mut request = create_room::v3::Request::new();
        request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
        request.is_direct = true;
        request.power_level_content_override = Some(matrix_sdk::ruma::serde::Raw::from_json(
            serde_json::value::to_raw_value(
                &serde_json::json!({"invite":100,"state_default":100}),
            )?,
        ));
        request.invite = vec![owner_id.clone()];
        // The owner receives only membership rights; only the bot can invite.
        let room = adapter.client.create_room(request).await?;
        save(&path.join("admin-room.json"), &room.room_id().as_str())?;
        room
    };
    // Fetch actual membership and encryption on every response, not stale sync
    // cache. A room marked direct is not by itself a privacy guarantee.
    let members = adapter
        .client
        .send(get_member_events::v3::Request::new(
            room.room_id().to_owned(),
        ))
        .await?;
    let mut has_owner = false;
    for event in members.chunk {
        let value: Value = serde_json::from_str(event.json().get())?;
        if matches!(
            value["content"]["membership"].as_str(),
            Some("join" | "invite")
        ) {
            let id = value["state_key"]
                .as_str()
                .ok_or("invalid private room membership")?;
            if id != own.as_str() && id != owner_id.as_str() {
                return Err("private admin room has another participant".into());
            }
            has_owner |= id == owner_id.as_str();
        }
    }
    if !has_owner {
        return Err("owner has left the private admin room".into());
    }
    // Crypto-disabled SDK send_raw does not protect encrypted rooms. Fetch state
    // ourselves before placing any private text on the wire.
    let states = adapter
        .client
        .send(
            matrix_sdk::ruma::api::client::state::get_state_events::v3::Request::new(
                room.room_id().to_owned(),
            ),
        )
        .await?;
    for event in states.room_state {
        let value: Value = serde_json::from_str(event.json().get())?;
        if value["type"] == "m.room.encryption" {
            return Err("encrypted rooms not supported yet".into());
        }
    }
    Ok(room)
}
async fn execute_command(
    bot: Arc<Bot>,
    adapter: MatrixAdapter,
    message: IncomingMessage,
    parsed: Result<Option<command::ActionArgs>, command::ActionError>,
    path: PathBuf,
) {
    if bot.stealth {
        return;
    }
    let owner = crate::owner_id().filter(|owner| owner.platform() == PlatformKind::Matrix);
    let private_id: Option<String> = match load(&path.join("admin-room.json")) {
        Ok(id) => id,
        Err(error) => {
            log::warn!("Matrix admin room state: {error}");
            return;
        }
    };
    let result = if owner != Some(&message.author.id) {
        Err(command::ActionError::PermissionDenied)
    } else {
        match parsed {
            Ok(Some(args)) => {
                AdminService::new(&bot)
                    .execute(
                        &Actor::User(message.author.id.clone()),
                        &ActionTarget {
                            connection: Some(adapter.connection.clone()),
                            room: if private_id.as_deref().is_some_and(|id| {
                                events::room_ref(id).is_ok_and(|room| room.id == message.room.id)
                            }) {
                                None
                            } else {
                                Some(message.room.clone())
                            },
                        },
                        args,
                    )
                    .await
            }
            Ok(None) => return,
            Err(error) => Err(error),
        }
    };
    let text = match result {
        Ok(result) => match command::render::plain(&result, true) {
            Ok(text) => text,
            Err(error) => command::render::localized_description(&error),
        },
        Err(error) => command::render::localized_description(&error),
    };
    let private = match owner.filter(|owner| *owner == &message.author.id) {
        Some(owner) => private_room(&adapter, owner, &path).await,
        None => Err("not the configured owner".into()),
    };
    match private {
        Ok(room) => {
            for part in render::render_final(
                crate::platform::outgoing::ThoughtPresentation::Off,
                "",
                &[text],
            ) {
                let body = OutgoingMessage::text(&part);
                if let Err(error) = room
                    .send_raw(
                        "m.room.message",
                        match render::content(&body) {
                            Ok(value) => value,
                            Err(error) => {
                                log::warn!("Matrix admin rendering: {error}");
                                return;
                            }
                        },
                    )
                    .await
                {
                    log::warn!("Matrix private admin delivery: {error}");
                }
            }
        }
        Err(error) => {
            log::warn!("Matrix private admin surface unavailable: {error}");
            let error = if owner == Some(&message.author.id) {
                command::ActionError::PrivateSurfaceRequired
            } else {
                command::ActionError::PermissionDenied
            };
            let text = command::render::localized_description(&error);
            if let Err(error) = adapter
                .send(&message.room, &OutgoingMessage::text(&text))
                .await
            {
                log::warn!("Matrix command notice: {error}");
            }
        }
    }
}
pub(crate) async fn run(
    bot: Arc<Bot>,
    connection: ConnectionId,
    config: MatrixConfig,
    ingress: EventSender,
    mut shutdown: ShutdownReceiver,
) -> Result<(), BotError> {
    if *shutdown.borrow() {
        return Ok(());
    }
    let path = state_path(&connection);
    let client = tokio::time::timeout(Duration::from_secs(30), login(&config, &path)).await??;
    let owner = crate::owner_id().filter(|owner| owner.platform() == PlatformKind::Matrix);
    let mut owner_approved: HashSet<RoomKey> = load(&path.join("rooms.json"))?.unwrap_or_default();
    let mut approved = owner_approved.clone();
    for room in &config.rooms {
        approved.insert(events::room_ref(room)?.id);
    }
    policy(&bot, &connection, &approved, owner)?;
    let cursor: Option<String> = load(&path.join("sync.json"))?;
    let settings = SyncSettings::default()
        .timeout(Duration::from_millis(0))
        .full_state(true);
    let settings = if let Some(cursor) = cursor {
        settings.token(cursor)
    } else {
        settings
    };
    let first = client.sync_once(settings).await?;
    let adapter = MatrixAdapter::new(client.clone(), connection.clone(), bot.image_analysis)?;
    bot.register_adapter(Adapter::Matrix(adapter.clone()))?;
    ingress
        .send(envelope(
            &connection,
            &format!("ready:{}", now()),
            ChatEvent::ConnectionReady(adapter.account.clone()),
        )?)
        .await?;
    for room in &config.rooms {
        if let Err(error) = client.join_room_by_id(&RoomId::parse(room)?).await {
            log::warn!("Matrix allowlisted room join: {error}");
        }
    }
    let mut response = first;
    let mut announced = HashSet::new();
    let mut encrypted_noticed = HashSet::new();
    // An incremental sync can omit unchanged rooms. Restore already-synced,
    // approved routes from the persisted SDK cache before waiting for new chat.
    let admin_room: Option<String> = load(&path.join("admin-room.json"))?;
    for native in client.joined_rooms() {
        if !native.is_state_fully_synced()
            || admin_room.as_deref() == Some(native.room_id().as_str())
        {
            continue;
        }
        let mut room = events::room_ref(native.room_id().as_str())?;
        if !approved.contains(&room.id) {
            continue;
        }
        room.name = native.name();
        let caps = adapter.refresh_capabilities(&room).await?;
        if native.encryption_state().is_encrypted() {
            encrypted_noticed.insert(room.id.clone());
            log::warn!(
                "Matrix room [{}]: {}",
                native.room_id(),
                crate::strings::t("matrix.encrypted_unsupported")
            );
            continue;
        }
        if caps.history == crate::platform::capabilities::HistoryAccess::None {
            continue;
        }
        bot.adapters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bind_discovered(&connection, room.clone())?;
        announced.insert(room.id.clone());
        let space = SpaceRef {
            id: SpaceKey::new(PlatformKind::Matrix, &[native.room_id().as_str()])?,
            name: room.name.clone(),
            rooms: vec![room.clone()],
            preferred_room: Some(room.id.clone()),
        };
        ingress
            .send(envelope(
                &connection,
                &format!("room-ready:{}:{}", native.room_id(), now()),
                ChatEvent::SpaceReady {
                    space,
                    history: HistoryInitializationPolicy::AfterSuccess,
                },
            )?)
            .await?;
    }
    let mut command_ids: std::collections::VecDeque<String> =
        load(&path.join("commands.json"))?.unwrap_or_default();
    let mut commands = JoinSet::new();
    let (command_tx, mut command_rx) = tokio::sync::mpsc::channel::<(
        IncomingMessage,
        Result<Option<command::ActionArgs>, command::ActionError>,
    )>(16);
    let command_bot = bot.clone();
    let command_adapter = adapter.clone();
    let command_path = path.clone();
    commands.spawn(async move {
        while let Some((message, parsed)) = command_rx.recv().await {
            execute_command(
                command_bot.clone(),
                command_adapter.clone(),
                message,
                parsed,
                command_path.clone(),
            )
            .await;
        }
    });
    loop {
        for (id, invite) in &response.rooms.invited {
            let room = events::room_ref(id.as_str())?;
            let mut inviter = None;
            let mut encrypted = false;
            for raw in &invite.invite_state.events {
                let value: Value = serde_json::from_str(raw.json().get())?;
                encrypted |= value["type"] == "m.room.encryption";
                if value["type"] == "m.room.member"
                    && value["state_key"].as_str() == client.user_id().map(UserId::as_str)
                    && value["content"]["membership"] == "invite"
                {
                    inviter = value["sender"]
                        .as_str()
                        .and_then(|id| events::user_ref(id, None).ok())
                        .map(|user| user.id);
                }
            }
            if auto_join(&room.id, inviter.as_ref(), owner, &approved) {
                if encrypted {
                    log::warn!(
                        "Matrix room [{id}]: {}",
                        crate::strings::t("matrix.encrypted_unsupported")
                    );
                    continue;
                }
                approved.insert(room.id.clone());
                if owner.is_some_and(|owner| inviter.as_ref() == Some(owner)) {
                    owner_approved.insert(room.id.clone());
                    save(&path.join("rooms.json"), &owner_approved)?;
                }
                policy(&bot, &connection, &approved, owner)?;
                if let Err(error) = client.join_room_by_id(id).await {
                    log::warn!("Matrix invited room join: {error}");
                }
            }
        }
        let admin_room: Option<String> = load(&path.join("admin-room.json"))?;
        for (id, update) in &response.rooms.joined {
            let mut room = events::room_ref(id.as_str())?;
            if admin_room.as_deref() == Some(id.as_str()) {
                for event in &update.timeline.events {
                    let raw: Value = serde_json::from_str(event.raw().json().get())?;
                    if let Ok(Some(ChatEvent::MessageCreated(message))) =
                        events::decode_event(&connection, &room, &adapter.account.id, &raw, false)
                    {
                        if let Some(text) = command_text(&message.text) {
                            let parsed = command::parser::parse_text(
                                &text,
                                &command::parser::CommandPrefix::Explicit("!bot".into()),
                            );
                            let event_id = raw["event_id"]
                                .as_str()
                                .ok_or("missing Matrix command event ID")?
                                .to_owned();
                            if command_ids.contains(&event_id) {
                                continue;
                            }
                            if command_tx.try_send((message, parsed)).is_err() {
                                log::warn!("Matrix command queue full");
                                continue;
                            }
                            command_ids.push_back(event_id);
                            if command_ids.len() > 256 {
                                command_ids.pop_front();
                            }
                            save(&path.join("commands.json"), &command_ids)?;
                        }
                    }
                }
                continue;
            }
            if !approved.contains(&room.id) {
                continue;
            }
            let native = client.get_room(id).ok_or("synced Matrix room missing")?;
            room.name = native.name();
            let caps = adapter.refresh_capabilities(&room).await?;
            if native.encryption_state().is_encrypted() {
                if encrypted_noticed.insert(room.id.clone()) {
                    log::warn!(
                        "Matrix room [{id}]: {}",
                        crate::strings::t("matrix.encrypted_unsupported")
                    );
                }
                continue;
            }
            if !caps.send && caps.history == crate::platform::capabilities::HistoryAccess::None {
                continue;
            }
            {
                let mut registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
                registry.bind_discovered(&connection, room.clone())?;
            }
            if announced.insert(room.id.clone()) {
                let space = SpaceRef {
                    id: SpaceKey::new(PlatformKind::Matrix, &[id.as_str()])?,
                    name: room.name.clone(),
                    rooms: vec![room.clone()],
                    preferred_room: Some(room.id.clone()),
                };
                ingress
                    .send(envelope(
                        &connection,
                        &format!("room-ready:{id}:{}", now()),
                        ChatEvent::SpaceReady {
                            space,
                            history: HistoryInitializationPolicy::AfterSuccess,
                        },
                    )?)
                    .await?;
            }
            ingress
                .send(envelope(
                    &connection,
                    &format!("capabilities:{id}:{}", response.next_batch),
                    ChatEvent::CapabilitiesChanged {
                        room: room.id.clone(),
                        capabilities: caps,
                    },
                )?)
                .await?;
            for event in &update.timeline.events {
                let raw: Value = serde_json::from_str(event.raw().json().get())?;
                let decoded = match events::decode_event(
                    &connection,
                    &room,
                    &adapter.account.id,
                    &raw,
                    bot.image_analysis,
                ) {
                    Ok(event) => event,
                    Err(error) => {
                        log::warn!("Matrix event decoding [{id}]: {error}");
                        continue;
                    }
                };
                let Some(mut decoded) = decoded else {
                    continue;
                };
                match &mut decoded {
                    ChatEvent::MessageCreated(message) => {
                        let sender =
                            UserId::parse(message.author.username.as_deref().unwrap_or("")).ok();
                        if let Some(sender) = sender {
                            if let Ok(Some(member)) = native.get_member_no_sync(&sender).await {
                                message.author.display_name =
                                    member.display_name().unwrap_or(sender.as_str()).to_owned();
                            }
                        }
                        if let Some(text) = command_text(&message.text) {
                            let event_id = raw["event_id"]
                                .as_str()
                                .ok_or("missing Matrix command event ID")?
                                .to_owned();
                            if command_ids.contains(&event_id) {
                                continue;
                            }
                            let parsed = command::parser::parse_text(
                                &text,
                                &command::parser::CommandPrefix::Explicit("!bot".into()),
                            );
                            if command_tx.try_send((message.clone(), parsed)).is_err() {
                                log::warn!("Matrix command queue full");
                                continue;
                            }
                            command_ids.push_back(event_id);
                            if command_ids.len() > 256 {
                                command_ids.pop_front();
                            }
                            save(&path.join("commands.json"), &command_ids)?;
                            continue;
                        }
                        if let Some(reply) = &mut message.reply_to {
                            if let Some(id) = reply.id.components().next().and_then(Result::ok) {
                                if let Ok(id) = matrix_sdk::ruma::EventId::parse(id) {
                                    if let Ok(target) = native.event(&id, None).await {
                                        let target: Value =
                                            serde_json::from_str(target.raw().json().get())?;
                                        reply.author = target["sender"]
                                            .as_str()
                                            .and_then(|id| events::user_ref(id, None).ok())
                                            .map(|user| user.id);
                                    }
                                }
                            }
                        }
                    }
                    ChatEvent::ReactionAdded(reaction) => {
                        if let Some(id) =
                            reaction.target.id.components().next().and_then(Result::ok)
                        {
                            if let Ok(id) = matrix_sdk::ruma::EventId::parse(id) {
                                if let Ok(target) = native.event(&id, None).await {
                                    let raw: Value =
                                        serde_json::from_str(target.raw().json().get())?;
                                    reaction.target.author = raw["sender"]
                                        .as_str()
                                        .and_then(|id| events::user_ref(id, None).ok())
                                        .map(|user| user.id);
                                    reaction.original_text =
                                        raw["content"]["body"].as_str().map(str::to_owned);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                let id = raw["event_id"].as_str().ok_or("missing Matrix event ID")?;
                ingress
                    .send(EventEnvelope {
                        connection: connection.clone(),
                        event_id: events::event_key(id)?,
                        occurred_at_unix_ms: raw["origin_server_ts"].as_i64().unwrap_or_else(now),
                        event: decoded,
                    })
                    .await?;
            }
            if update.timeline.limited {
                log::warn!("Matrix sync gap [{id}]: history is bounded; events outside synced timeline may not be observed");
            }
        }
        save(&path.join("sync.json"), &response.next_batch)?;
        let settings = SyncSettings::default()
            .timeout(Duration::from_secs(30))
            .token(response.next_batch.clone());
        loop {
            tokio::select! {
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { commands.abort_all(); return Ok(()); } },
                result = commands.join_next(), if !commands.is_empty() => { if let Some(Err(error)) = result { log::warn!("Matrix admin task: {error}"); } },
                result = client.sync_once(settings.clone()) => match result {
                    Ok(next) => {response = next; break;},
                    Err(error) => {
                        log::warn!("Matrix sync disconnected: {error}");
                        tokio::select! { _ = shutdown.changed() => {commands.abort_all(); return Ok(());}, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_join_is_owner_or_explicit_room_only() {
        let room = events::room_ref("!room:example.org").unwrap().id;
        let owner = events::user_ref("@owner:example.org", None).unwrap().id;
        let stranger = events::user_ref("@stranger:example.org", None).unwrap().id;
        let mut allowed = HashSet::new();
        assert!(!auto_join(&room, Some(&stranger), Some(&owner), &allowed));
        assert!(!auto_join(&room, None, None, &allowed));
        assert!(auto_join(&room, Some(&owner), Some(&owner), &allowed));
        allowed.insert(room.clone());
        assert!(auto_join(&room, Some(&stranger), None, &allowed));
    }
    #[test]
    fn commands_are_explicit_and_use_shared_parser() {
        assert_eq!(command_text("ordinary text"), None);
        assert_eq!(command_text("!status"), Some("!bot status".into()));
        assert_eq!(command_text("!bot status"), Some("!bot status".into()));
        assert_eq!(command_text("! status"), None);
        assert_eq!(command_text("!random-exclamation"), None);
        assert_eq!(command_text("!bot unknown"), Some("!bot unknown".into()));
    }
    #[tokio::test]
    #[ignore = "requires a disposable Matrix homeserver/account; see setup guide"]
    async fn matrix_live_session_survives_restart() {
        let (Ok(homeserver), Ok(user), Ok(password)) = (
            std::env::var("MATRIX_TEST_HOMESERVER"),
            std::env::var("MATRIX_TEST_USER"),
            std::env::var("MATRIX_TEST_PASSWORD"),
        ) else {
            return;
        };
        let path = std::env::temp_dir().join(format!(
            "matrix-live-session-{}-{}",
            std::process::id(),
            now()
        ));
        let config = MatrixConfig {
            homeserver,
            user,
            password: Some(password),
            access_token: None,
            rooms: vec![],
        };
        let first = login(&config, &path).await.unwrap();
        let device = first.device_id().unwrap().to_owned();
        let token = first.access_token().unwrap();
        first
            .sync_once(SyncSettings::default().timeout(Duration::from_secs(0)))
            .await
            .unwrap();
        drop(first);
        let second = login(&config, &path).await.unwrap();
        assert_eq!(second.device_id().unwrap(), device);
        assert_eq!(second.access_token().unwrap(), token);
        drop(second);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    #[ignore = "creates and encrypts a disposable room; see setup guide"]
    async fn matrix_live_plaintext_transport_and_encryption_rejection() {
        use crate::platform::outgoing::{HistoryRequest, NickTarget};
        let (Ok(homeserver), Ok(user), Ok(password)) = (
            std::env::var("MATRIX_TEST_HOMESERVER"),
            std::env::var("MATRIX_TEST_USER"),
            std::env::var("MATRIX_TEST_PASSWORD"),
        ) else {
            return;
        };
        let path = std::env::temp_dir().join(format!(
            "matrix-live-transport-{}-{}",
            std::process::id(),
            now()
        ));
        let config = MatrixConfig {
            homeserver,
            user,
            password: Some(password),
            access_token: None,
            rooms: vec![],
        };
        let client = login(&config, &path).await.unwrap();
        let native = client
            .create_room(create_room::v3::Request::new())
            .await
            .unwrap();
        client
            .sync_once(
                SyncSettings::default()
                    .timeout(Duration::ZERO)
                    .full_state(true),
            )
            .await
            .unwrap();
        let room = events::room_ref(native.room_id().as_str()).unwrap();
        let adapter =
            MatrixAdapter::new(client.clone(), ConnectionId::new("matrix-live"), false).unwrap();
        let caps = adapter.refresh_capabilities(&room).await.unwrap();
        assert!(caps.send && caps.react && caps.edit && caps.delete && caps.typing);
        assert!(!caps.decryption_available);
        let receipt = adapter
            .send(&room, &OutgoingMessage::text("live transport message"))
            .await
            .unwrap();
        adapter
            .edit(
                &receipt,
                &OutgoingMessage::text("edited live transport message"),
            )
            .await
            .unwrap();
        adapter
            .react(&receipt.message, &Emoji::Unicode("👍".into()))
            .await
            .unwrap();
        adapter.typing(&room).await.unwrap();
        adapter
            .set_nickname(NickTarget::Room(&room), "Plaintext test bot")
            .await
            .unwrap();
        adapter
            .fetch_history(&room, HistoryRequest::default())
            .await
            .unwrap();
        adapter.delete(&receipt).await.unwrap();
        native
            .send_state_event_raw(
                "m.room.encryption",
                "",
                serde_json::json!({"algorithm":"m.megolm.v1.aes-sha2"}),
            )
            .await
            .unwrap();
        client
            .sync_once(
                SyncSettings::default()
                    .timeout(Duration::ZERO)
                    .full_state(true),
            )
            .await
            .unwrap();
        let caps = adapter.refresh_capabilities(&room).await.unwrap();
        assert!(!caps.send && !caps.decryption_available);
        assert!(matches!(
            adapter
                .send(&room, &OutgoingMessage::text("must not be sent"))
                .await,
            Err(crate::platform::PlatformError::Unsupported(
                "encrypted rooms not supported yet"
            ))
        ));
        native.leave().await.unwrap();
        drop(adapter);
        drop(native);
        drop(client);
        std::fs::remove_dir_all(path).unwrap();
    }
}

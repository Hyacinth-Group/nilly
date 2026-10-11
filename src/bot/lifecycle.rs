impl Bot {
    /// Refresh a connection without replacing the shared personality on reconnect.
    pub(crate) fn register_adapter(&self, adapter: platform::Adapter)
        -> platform::PlatformResult<Arc<platform::Adapter>>
    {
        let connection = adapter.connection_id().clone();
        let account = adapter.self_user().clone();
        let mut registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        registry.configure(&connection, account.id.platform())?;
        if account.id.platform() == PlatformKind::Discord {
            registry.set_room_policy(&connection, platform::registry::RoomPolicy::LegacyDiscord {
                space: self.guild_id.clone(),
                rooms: self.allowed_channels.clone(),
            })?;
        }
        let adapter = registry.register(adapter);
        if account.id.platform() == PlatformKind::Discord {
            for room in [&self.news_channel, &self.debug_channel].into_iter().flatten() {
                registry.bind(&connection, RoomRef::from_key(room.clone()))?;
            }
        }
        drop(registry);
        update_connection_account(&mut self.state(), connection, account);
        Ok(adapter)
    }

    /// Common ingress only consumes domain events, never SDK objects or caches.
    pub(crate) async fn handle_event(self: &Arc<Self>, envelope: EventEnvelope) {
        let platform = self.adapters.lock().unwrap_or_else(|e| e.into_inner())
            .configured_platform(&envelope.connection);
        if !platform.is_some_and(|platform| valid_envelope_source(&envelope, platform)) {
            log::warn!("ingress [{}]: invalid event source identity", envelope.connection);
            return;
        }
        let connection = envelope.connection;
        match envelope.event {
            ChatEvent::ConnectionClosed => {
                if let Err(error) = self.adapters.lock().unwrap_or_else(|e| e.into_inner())
                    .set_status(&connection, platform::outgoing::ConnectionStatus::Disconnected)
                {
                    log::warn!("connection [{connection}]: {error}");
                }
            }
            ChatEvent::ConnectionReady(account) => {
                let valid = {
                    let registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
                    registry.get(&connection).is_some_and(|adapter| adapter.self_user().id == account.id)
                };
                if !valid {
                    log::warn!("connection [{connection}]: ready without matching registered account");
                    return;
                }
                update_connection_account(&mut self.state(), connection, account);
                // Native UI registration completes before this event is submitted.
                if claim_once(&self.cycles_started) {
                    let bot = self.clone();
                    run_cycle("news", move || news_cycle(bot.clone()));
                    let bot = self.clone();
                    run_cycle("poke", move || poke_cycle(bot.clone()));
                    let bot = self.clone();
                    run_cycle("prank", move || prank_cycle(bot.clone()));
                    let bot = self.clone();
                    run_cycle("wanderer", move || wanderer_cycle(bot.clone()));
                    let bot = self.clone();
                    run_cycle("memory", move || memory_cycle(bot.clone()));
                    let bot = self.clone();
                    run_cycle("sleep", move || sleep_cycle(bot.clone()));
                }
            }
            ChatEvent::SpaceReady { space, history } => {
                self.space_ready(connection, space, history).await;
            }
            ChatEvent::RoomReady(room) => {
                if let Err(error) = self.adapters.lock().unwrap_or_else(|e| e.into_inner())
                    .bind_discovered(&connection, room)
                {
                    log::warn!("room discovery [{connection}]: {error}");
                }
            }
            ChatEvent::MessageCreated(message) => {
                if self.bind_incoming_room(&connection, &message.room) {
                    self.message_created(&connection, message).await;
                }
            }
            ChatEvent::ReactionAdded(event) => {
                if self.bind_incoming_room(&connection, &event.room) {
                    self.reaction_added(&connection, event).await;
                }
            }
            ChatEvent::MemberJoined { room, user } => {
                if self.bind_member_room(&connection, &room) {
                    self.member_joined(&connection, room, user).await;
                }
            }
            ChatEvent::CommandInvoked(_) => {
                // Native command surfaces acknowledge and invoke AdminService directly.
                log::warn!("command [{connection}]: {}", platform::PlatformError::Unsupported("normalized command ingress"));
            }
            ChatEvent::CapabilitiesChanged { room, capabilities } => {
                // Usability is sampled from the current adapter at each operation.
                log::debug!("capabilities [{connection}/{room}]: {capabilities:?}");
            }
        }
    }

    fn bind_incoming_room(&self, connection: &ConnectionId, room: &RoomRef) -> bool {
        let mut registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        match registry.bind_discovered(connection, room.clone()) {
            Ok(approved) => approved,
            Err(error) => {
                log::warn!("incoming room [{connection}/{}]: {error}", room.conversation());
                false
            }
        }
    }

    fn member_room_allowed(&self, connection: &ConnectionId, room: &RoomRef) -> bool {
        let registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        registry.allows(connection, room)
            || legacy_system_welcome(&registry, connection, room, self.guild_id.as_ref())
            || legacy_news_welcome(room, self.news_channel.as_ref(), registry.is_bound(connection, room))
    }

    fn bind_member_room(&self, connection: &ConnectionId, room: &RoomRef) -> bool {
        let mut registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        if !registry.allows(connection, room)
            && !legacy_system_welcome(&registry, connection, room, self.guild_id.as_ref())
            && !legacy_news_welcome(room, self.news_channel.as_ref(), registry.is_bound(connection, room))
        {
            return false;
        }
        match registry.bind(connection, room.clone()) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("member room [{connection}/{}]: {error}", room.conversation());
                false
            }
        }
    }

    async fn space_ready(self: &Arc<Self>, connection: ConnectionId, space: SpaceRef,
        policy: HistoryInitializationPolicy)
    {
        let approved = {
            let mut registry = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
            if registry.get(&connection).is_none() {
                return;
            }
            if let Err(error) = registry.record_space(&connection, space.clone()) {
                log::warn!("space discovery [{connection}/{}]: {error}", space.id);
                return;
            }
            let mut approved = false;
            for room in &space.rooms {
                match registry.bind_discovered(&connection, room.clone()) {
                    Ok(bound) => approved |= bound,
                    Err(error) => log::warn!("room discovery [{connection}/{}]: {error}", room.conversation()),
                }
            }
            if let Some(preferred) = &space.preferred_room {
                let route = ConversationKey { room: preferred.clone(), thread: None };
                let room = space.rooms.iter().find(|room| &room.id == preferred)
                    .cloned().or_else(|| registry.room(&route));
                if let Some(room) = room {
                    match registry.bind_discovered(&connection, room) {
                        Ok(bound) => approved |= bound,
                        Err(error) => log::warn!("preferred room [{connection}/{preferred}]: {error}"),
                    }
                }
            }
            approved
        };
        let legacy_space = policy == HistoryInitializationPolicy::AttemptOnce
            && space.id.platform() == PlatformKind::Discord
            && self.guild_id.as_ref().is_none_or(|allowed| allowed == &space.id);
        if !approved && !legacy_space {
            return;
        }
        let first_time = {
            let mut state = self.state();
            let first = begin_history(&mut state, &space.id, policy);
            if first && policy == HistoryInitializationPolicy::AttemptOnce {
                persist_scanned(&state);
            }
            first
        };
        if !self.stealth && claim_once(&self.announcement_sent) {
            if let Some(channel) = default_channel(self) {
                let (model, mode) = {
                    let state = self.state();
                    (state.model.clone(), state.thinking_mode.label())
                };
                let description = strings::t("announce.description")
                    .replace("{model}", &model).replace("{mode}", mode);
                let title = format!("{} · {}", strings::t("announce.title"), version_text());
                self.send_card(&channel, &title, &description).await;
            }
        }
        if !first_time {
            return;
        }
        let bot = self.clone();
        tokio::spawn(async move {
            let success = read_history(&bot, &connection, &space).await;
            {
                let mut state = bot.state();
                if finish_history(&mut state, &space.id, policy, success) {
                    persist_scanned(&state);
                }
            }
            // Discord historically profiles after its one attempted scan even on
            // fetch failure; other adapters never claim unsupported history succeeded.
            if success || policy == HistoryInitializationPolicy::AttemptOnce {
                bot.profiler().await;
                if bot.state().temperament.is_empty() {
                    bot.coach().await;
                }
            }
        });
    }
}

fn claim_once(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::SeqCst)
}

fn update_connection_account(state: &mut State, connection: ConnectionId, account: UserRef) {
    if state.primary_connection.is_none() {
        state.bot_name = state.growth.name.clone().unwrap_or_else(||
            account.username.clone().unwrap_or_else(|| account.display_name.clone()));
        state.primary_connection = Some(connection.clone());
    }
    state.accounts.insert(connection, account);
}

fn begin_history(state: &mut State, space: &SpaceKey, policy: HistoryInitializationPolicy) -> bool {
    if state.scanned.contains(space) || !state.history_in_progress.insert(space.clone()) {
        return false;
    }
    if policy == HistoryInitializationPolicy::AttemptOnce {
        state.scanned.insert(space.clone());
    }
    true
}

fn finish_history(state: &mut State, space: &SpaceKey, policy: HistoryInitializationPolicy,
    success: bool) -> bool
{
    state.history_in_progress.remove(space);
    policy == HistoryInitializationPolicy::AfterSuccess && success && state.scanned.insert(space.clone())
}

fn persist_scanned(state: &State) {
    let mut lines: Vec<_> = state.scanned.iter().map(ToString::to_string).collect();
    lines.sort();
    // Retain unresolved historical records without treating them as identities.
    lines.extend(memory::read("taranan.md").lines()
        .filter(|line| platform::keys::parse_space_line(line).is_none())
        .map(ToString::to_string));
    memory::write("taranan.md", &lines.join("\n"));
}

#[cfg(test)]
mod generic_lifecycle_tests {
    use super::*;

    fn account(id: u64, name: &str) -> UserRef {
        UserRef {
            id: UserKey::discord(id),
            display_name: name.into(),
            username: Some(name.into()),
            automation: platform::types::Automation::Bot,
            stability: platform::types::IdentityStability::Persistent,
        }
    }

    #[test]
    fn legacy_scan_marker_precedes_attempt_and_survives_failure_and_reconnect() {
        let mut state = State::default();
        let space = SpaceKey::discord(9);
        assert!(begin_history(&mut state, &space, HistoryInitializationPolicy::AttemptOnce));
        assert!(state.scanned.contains(&space));
        assert!(!begin_history(&mut state, &space, HistoryInitializationPolicy::AttemptOnce));
        assert!(!finish_history(&mut state, &space, HistoryInitializationPolicy::AttemptOnce, false));
        assert!(state.history_in_progress.is_empty());
        assert!(!begin_history(&mut state, &space, HistoryInitializationPolicy::AttemptOnce));
    }

    #[test]
    fn success_policy_does_not_mark_unsupported_or_failed_scans() {
        let mut state = State::default();
        let space = SpaceKey::new(PlatformKind::Matrix, &["!room:example.org"]).unwrap();
        assert!(begin_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess));
        assert!(!state.scanned.contains(&space));
        assert!(!begin_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess));
        assert!(!finish_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess, false));
        assert!(!state.scanned.contains(&space));
        assert!(begin_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess));
        assert!(finish_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess, true));
        assert!(state.scanned.contains(&space));
        assert!(!begin_history(&mut state, &space, HistoryInitializationPolicy::AfterSuccess));
    }

    #[test]
    fn reconnect_and_secondary_accounts_never_replace_primary_personality_name() {
        let mut state = State::default();
        let first = ConnectionId::new("first");
        update_connection_account(&mut state, first.clone(), account(1, "Primary"));
        update_connection_account(&mut state, ConnectionId::new("second"), account(2, "Secondary"));
        update_connection_account(&mut state, first.clone(), account(1, "Renamed account"));
        assert_eq!(state.bot_name, "Primary");
        assert_eq!(state.primary_connection, Some(first.clone()));
        assert_eq!(state.accounts[&first].display_name, "Renamed account");
        assert_eq!(state.accounts.len(), 2);
    }

    #[test]
    fn persisted_chosen_name_precedes_first_connection_account() {
        let mut state = State::default();
        state.growth.name = Some("Chosen".into());
        update_connection_account(&mut state, ConnectionId::new("first"), account(1, "Account"));
        assert_eq!(state.bot_name, "Chosen");
    }

    #[test]
    fn shared_cycle_claim_is_once_across_connections_and_reconnects() {
        let flag = Arc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..8).map(|_| {
            let flag = flag.clone();
            std::thread::spawn(move || claim_once(&flag))
        }).collect();
        let winners = threads.into_iter().map(|thread| thread.join().unwrap())
            .filter(|claimed| *claimed).count();
        assert_eq!(winners, 1);
        assert!(!claim_once(&flag));
    }
}

#[cfg(test)]
mod generic_disconnect_tests {
    use super::*;

    #[test]
    fn disconnected_connection_retains_configuration_without_dispatch_or_discovery() {
        let mut registry = platform::ConnectionRegistry::default();
        let connection = ConnectionId::new("offline");
        registry.configure(&connection, PlatformKind::Discord).unwrap();
        let room = RoomRef {
            space: Some(SpaceKey::discord(9)),
            ..RoomRef::from_key(RoomKey::discord(7))
        };
        registry.record_space(&connection, SpaceRef {
            id: SpaceKey::discord(9),
            name: Some("Known space".into()),
            rooms: vec![room.clone()],
            preferred_room: Some(room.id.clone()),
        }).unwrap();
        registry.set_status(&connection, platform::outgoing::ConnectionStatus::Disconnected).unwrap();
        assert!(registry.get(&connection).is_none());
        assert!(registry.for_conversation(&room.conversation()).is_none());
        assert!(!registry.is_bound(&connection, &room));
        assert!(registry.discovered_spaces().is_empty());
        assert!(matches!(registry.bind_discovered(&connection, room),
            Err(platform::PlatformError::Disconnected)));
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].status, platform::outgoing::ConnectionStatus::Disconnected);
        assert_eq!(snapshot[0].connection, connection);
    }
}

fn legacy_system_welcome(
    registry: &platform::ConnectionRegistry,
    connection: &ConnectionId,
    room: &RoomRef,
    configured_space: Option<&SpaceKey>,
) -> bool {
    room.id.platform() == PlatformKind::Discord
        && room.thread.is_none()
        && registry.configured_platform(connection) == Some(PlatformKind::Discord)
        && room.space.as_ref().is_some_and(|room_space|
            configured_space.is_none_or(|allowed| allowed == room_space)
                && registry.discovered_spaces().iter().any(|(source, space)|
                    source == connection && &space.id == room_space
                        && space.preferred_room.as_ref() == Some(&room.id)))
}

fn legacy_news_welcome(room: &RoomRef, configured_news: Option<&RoomKey>, bound: bool) -> bool {
    // Native member decoding already enforces the joining guild policy. Its
    // explicit news fallback can intentionally target another guild.
    bound && room.id.platform() == PlatformKind::Discord && room.thread.is_none()
        && configured_news == Some(&room.id)
}

/// All keys in an ingress payload must originate from its configured transport.
/// This check precedes discovery/binding and any memory or model work.
fn valid_envelope_source(envelope: &EventEnvelope, platform: PlatformKind) -> bool {
    if envelope.event_id.platform() != platform {
        return false;
    }
    if platform == PlatformKind::Slack {
        let scope = match &envelope.event {
            ChatEvent::ConnectionReady(account) => Some(account.id.as_str()),
            ChatEvent::ConnectionClosed => None,
            ChatEvent::SpaceReady { space, .. } => Some(space.id.as_str()),
            ChatEvent::RoomReady(room) | ChatEvent::MemberJoined { room, .. } => Some(room.id.as_str()),
            ChatEvent::MessageCreated(message) => Some(message.room.id.as_str()),
            ChatEvent::ReactionAdded(event) => Some(event.room.id.as_str()),
            ChatEvent::CommandInvoked(command) => Some(command.room.id.as_str()),
            ChatEvent::CapabilitiesChanged { room, .. } => Some(room.as_str()),
        };
        if scope.is_some_and(|scope| !same_native_prefix(envelope.event_id.as_str(), scope, 1)) {
            return false;
        }
    }
    match &envelope.event {
        ChatEvent::ConnectionReady(account) => account.id.platform() == platform,
        ChatEvent::ConnectionClosed => true,
        ChatEvent::SpaceReady { space, .. } =>
            space.id.platform() == platform
                && space.preferred_room.as_ref().is_none_or(|room|
                    room.platform() == platform && space_matches_room(&space.id, room, platform))
                && space.rooms.iter().all(|room| valid_room_source(room, platform)
                    && space_matches_room(&space.id, &room.id, platform)
                    && room.space.as_ref().is_none_or(|id| id == &space.id)),
        ChatEvent::RoomReady(room) => valid_room_source(room, platform),
        ChatEvent::MessageCreated(message) =>
            valid_message_source(message, &envelope.connection, platform),
        ChatEvent::ReactionAdded(event) =>
            valid_room_source(&event.room, platform)
                && user_matches_room(&event.actor.id, &event.room.id, platform)
                && valid_message_ref_source(&event.target, platform)
                && event.target.room == event.room.id
                && match &event.emoji {
                    platform::types::Emoji::Unicode(_) => true,
                    platform::types::Emoji::Custom { platform: source, .. } => *source == platform,
                },
        ChatEvent::MemberJoined { room, user } =>
            valid_room_source(room, platform) && user_matches_room(&user.id, &room.id, platform),
        ChatEvent::CommandInvoked(command) =>
            valid_room_source(&command.room, platform)
                && user_matches_room(&command.actor.id, &command.room.id, platform),
        ChatEvent::CapabilitiesChanged { room, .. } => room.platform() == platform,
    }
}

fn valid_room_source(room: &RoomRef, platform: PlatformKind) -> bool {
    room.id.platform() == platform
        && room.space.as_ref().is_none_or(|space|
            space.platform() == platform && space_matches_room(space, &room.id, platform))
        && room.thread.as_ref().is_none_or(|thread|
            thread.platform() == platform && native_message_matches_room(thread.as_str(), &room.id, platform))
}

fn valid_message_ref_source(message: &platform::types::MessageRef, platform: PlatformKind) -> bool {
    message.id.platform() == platform && message.room.platform() == platform
        && native_message_matches_room(message.id.as_str(), &message.room, platform)
        && message.author.as_ref().is_none_or(|author| user_matches_room(author, &message.room, platform))
}

fn valid_message_source(message: &IncomingMessage, connection: &ConnectionId,
    platform: PlatformKind) -> bool
{
    message.id.platform() == platform
        && native_message_matches_room(message.id.as_str(), &message.room.id, platform)
        && valid_room_source(&message.room, platform)
        && user_matches_room(&message.author.id, &message.room.id, platform)
        && message.mentions.iter().all(|user| user_matches_room(user, &message.room.id, platform))
        && message.reply_to.as_ref().is_none_or(|reply|
            valid_message_ref_source(reply, platform) && reply.room == message.room.id)
        && message.attachments.iter().all(|attachment| match &attachment.source {
            platform::types::MediaSource::PublicUrl(_) => true,
            platform::types::MediaSource::Native { connection: source, .. } => source == connection,
        })
}

fn same_native_prefix(first: &str, second: &str, components: usize) -> bool {
    first.split(':').skip(2).take(components)
        .eq(second.split(':').skip(2).take(components))
}

fn user_matches_room(user: &UserKey, room: &RoomKey, platform: PlatformKind) -> bool {
    user.platform() == platform && (platform != PlatformKind::Slack
        || same_native_prefix(user.as_str(), room.as_str(), 1))
}

fn space_matches_room(space: &SpaceKey, room: &RoomKey, platform: PlatformKind) -> bool {
    platform != PlatformKind::Slack || same_native_prefix(space.as_str(), room.as_str(), 1)
}

fn native_message_matches_room(message: &str, room: &RoomKey, platform: PlatformKind) -> bool {
    match platform {
        PlatformKind::Slack => same_native_prefix(message, room.as_str(), 2),
        PlatformKind::Telegram => same_native_prefix(message, room.as_str(), 1),
        PlatformKind::Discord | PlatformKind::Matrix | PlatformKind::Terminal => true,
    }
}

#[cfg(test)]
mod generic_ingress_source_tests {
    use super::*;

    fn message() -> IncomingMessage {
        IncomingMessage {
            id: MessageKey::discord(1),
            room: RoomRef {
                space: Some(SpaceKey::discord(9)),
                ..RoomRef::from_key(RoomKey::discord(7))
            },
            author: UserRef {
                id: UserKey::discord(2),
                display_name: "Alice".into(),
                username: None,
                automation: platform::types::Automation::Human,
                stability: platform::types::IdentityStability::Persistent,
            },
            sent_at_unix_ms: 1,
            text: "hello".into(),
            mentions: Vec::new(),
            reply_to: None,
            attachments: Vec::new(),
        }
    }

    fn envelope(event: ChatEvent) -> EventEnvelope {
        EventEnvelope {
            connection: ConnectionId::new("source"),
            event_id: platform::types::EventKey::discord(1),
            occurred_at_unix_ms: 1,
            event,
        }
    }

    #[test]
    fn foreign_message_identity_components_and_media_connections_are_rejected() {
        let foreign_user = || UserKey::new(PlatformKind::Matrix, &["@other:example.org"]).unwrap();
        let foreign_room = || RoomKey::new(PlatformKind::Matrix, &["!room:example.org"]).unwrap();
        let foreign_message = || MessageKey::new(PlatformKind::Matrix, &["$event"]).unwrap();
        let baseline = message();
        assert!(valid_envelope_source(&envelope(ChatEvent::MessageCreated(baseline.clone())),
            PlatformKind::Discord));
        let mut cases = Vec::new();
        let mut invalid = baseline.clone();
        invalid.id = foreign_message();
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.author.id = foreign_user();
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.room.id = foreign_room();
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.room.space = Some(SpaceKey::new(PlatformKind::Matrix, &["!space:example.org"]).unwrap());
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.room.thread = Some(platform::types::ThreadKey::new(PlatformKind::Matrix, &["$thread"]).unwrap());
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.mentions.push(foreign_user());
        cases.push(invalid);
        for target in [
            platform::types::MessageRef { id: foreign_message(), room: RoomKey::discord(7), author: None },
            platform::types::MessageRef { id: MessageKey::discord(3), room: foreign_room(), author: None },
            platform::types::MessageRef { id: MessageKey::discord(3), room: RoomKey::discord(7), author: Some(foreign_user()) },
            platform::types::MessageRef { id: MessageKey::discord(3), room: RoomKey::discord(8), author: None },
        ] {
            let mut invalid = baseline.clone();
            invalid.reply_to = Some(target);
            cases.push(invalid);
        }
        let mut invalid = baseline;
        invalid.attachments.push(platform::types::AttachmentRef {
            kind: platform::types::MediaKind::Image,
            mime: None,
            size: None,
            source: platform::types::MediaSource::Native {
                connection: ConnectionId::new("other-credentials"),
                id: "media".into(),
            },
        });
        cases.push(invalid);
        for invalid in cases {
            assert!(!valid_envelope_source(&envelope(ChatEvent::MessageCreated(invalid)),
                PlatformKind::Discord));
        }
        let mut invalid = envelope(ChatEvent::ConnectionClosed);
        invalid.event_id = platform::types::EventKey::new(PlatformKind::Matrix, &["$event"]).unwrap();
        assert!(!valid_envelope_source(&invalid, PlatformKind::Discord));
    }

    #[test]
    fn foreign_members_reactions_and_space_metadata_are_rejected() {
        let baseline = message();
        let mut foreign = baseline.author.clone();
        foreign.id = UserKey::new(PlatformKind::Matrix, &["@other:example.org"]).unwrap();
        assert!(!valid_envelope_source(&envelope(ChatEvent::MemberJoined {
            room: baseline.room.clone(), user: foreign.clone(),
        }), PlatformKind::Discord));
        let mut reaction = platform::types::ReactionEvent {
            room: baseline.room.clone(),
            actor: baseline.author,
            target: platform::types::MessageRef {
                id: MessageKey::discord(1), room: RoomKey::discord(7), author: Some(UserKey::discord(2)),
            },
            emoji: platform::types::Emoji::Unicode("🙂".into()),
            original_text: None,
        };
        assert!(valid_envelope_source(&envelope(ChatEvent::ReactionAdded(reaction.clone())),
            PlatformKind::Discord));
        reaction.actor = foreign;
        assert!(!valid_envelope_source(&envelope(ChatEvent::ReactionAdded(reaction.clone())),
            PlatformKind::Discord));
        reaction.actor.id = UserKey::discord(2);
        reaction.emoji = platform::types::Emoji::Custom {
            platform: PlatformKind::Matrix, id: "emoji".into(), label: None, animated: false,
        };
        assert!(!valid_envelope_source(&envelope(ChatEvent::ReactionAdded(reaction)),
            PlatformKind::Discord));
        let mut space = SpaceRef {
            id: SpaceKey::discord(9), name: None,
            rooms: vec![baseline.room], preferred_room: Some(RoomKey::discord(7)),
        };
        space.rooms[0].space = Some(SpaceKey::discord(10));
        assert!(!valid_envelope_source(&envelope(ChatEvent::SpaceReady {
            space, history: HistoryInitializationPolicy::AttemptOnce,
        }), PlatformKind::Discord));
    }

    #[test]
    fn legacy_welcome_exception_is_only_discovered_system_room_in_configured_guild() {
        let connection = ConnectionId::new("source");
        let mut registry = platform::ConnectionRegistry::default();
        registry.configure(&connection, PlatformKind::Discord).unwrap();
        registry.set_room_policy(&connection, platform::registry::RoomPolicy::LegacyDiscord {
            space: Some(SpaceKey::discord(9)),
            rooms: Some(HashSet::from([RoomKey::discord(8)])),
        }).unwrap();
        let room = message().room;
        registry.record_space(&connection, SpaceRef {
            id: SpaceKey::discord(9), name: None, rooms: vec![room.clone()],
            preferred_room: Some(room.id.clone()),
        }).unwrap();
        registry.set_status(&connection, platform::outgoing::ConnectionStatus::Ready).unwrap();
        assert!(!registry.allows(&connection, &room));
        assert!(legacy_system_welcome(&registry, &connection, &room, Some(&SpaceKey::discord(9))));
        assert!(!legacy_system_welcome(&registry, &connection, &room, Some(&SpaceKey::discord(10))));
        let mut ordinary = room.clone();
        ordinary.id = RoomKey::discord(11);
        assert!(!legacy_system_welcome(&registry, &connection, &ordinary, None));
        assert!(!legacy_system_welcome(&registry, &ConnectionId::new("other"), &room, None));
        registry.set_status(&connection, platform::outgoing::ConnectionStatus::Disconnected).unwrap();
        assert!(!legacy_system_welcome(&registry, &connection, &room, None));
    }
}

#[cfg(test)]
mod generic_news_welcome_tests {
    use super::*;

    #[test]
    fn explicit_ready_news_welcome_does_not_reapply_joining_guild_to_output_route() {
        let room = RoomRef {
            space: Some(SpaceKey::discord(20)),
            ..RoomRef::from_key(RoomKey::discord(7))
        };
        assert!(legacy_news_welcome(&room, Some(&RoomKey::discord(7)), true));
        assert!(!legacy_news_welcome(&room, Some(&RoomKey::discord(7)), false));
        assert!(!legacy_news_welcome(&room, Some(&RoomKey::discord(8)), true));
        assert!(!legacy_news_welcome(&room, None, true));
        let mut topic = room;
        topic.thread = Some(platform::types::ThreadKey::discord(1));
        assert!(!legacy_news_welcome(&topic, Some(&RoomKey::discord(7)), true));
    }
}

#[cfg(test)]
mod generic_native_scope_tests {
    use super::*;

    fn slack_message() -> IncomingMessage {
        IncomingMessage {
            id: MessageKey::new(PlatformKind::Slack, &["T1", "C1", "1700000000.000123"]).unwrap(),
            room: RoomRef {
                id: RoomKey::new(PlatformKind::Slack, &["T1", "C1"]).unwrap(),
                space: Some(SpaceKey::new(PlatformKind::Slack, &["T1"]).unwrap()),
                kind: platform::types::RoomKind::Channel,
                name: None,
                thread: Some(platform::types::ThreadKey::new(
                    PlatformKind::Slack, &["T1", "C1", "1700000000.000001"]).unwrap()),
            },
            author: UserRef {
                id: UserKey::new(PlatformKind::Slack, &["T1", "U1"]).unwrap(),
                display_name: "Alice".into(),
                username: None,
                automation: platform::types::Automation::Human,
                stability: platform::types::IdentityStability::Persistent,
            },
            sent_at_unix_ms: 1,
            text: "hello".into(),
            mentions: vec![UserKey::new(PlatformKind::Slack, &["T1", "U2"]).unwrap()],
            reply_to: Some(platform::types::MessageRef {
                id: MessageKey::new(PlatformKind::Slack, &["T1", "C1", "1700000000.000100"]).unwrap(),
                room: RoomKey::new(PlatformKind::Slack, &["T1", "C1"]).unwrap(),
                author: Some(UserKey::new(PlatformKind::Slack, &["T1", "U2"]).unwrap()),
            }),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn same_platform_wrong_slack_workspaces_and_channels_are_rejected() {
        let connection = ConnectionId::new("source");
        let baseline = slack_message();
        assert!(valid_message_source(&baseline, &connection, PlatformKind::Slack));
        let mut cases = Vec::new();
        let mut invalid = baseline.clone();
        invalid.author.id = UserKey::new(PlatformKind::Slack, &["T2", "U1"]).unwrap();
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.mentions = vec![UserKey::new(PlatformKind::Slack, &["T2", "U2"]).unwrap()];
        cases.push(invalid);
        for (workspace, channel) in [("T2", "C1"), ("T1", "C2")] {
            let mut invalid = baseline.clone();
            invalid.id = MessageKey::new(PlatformKind::Slack, &[workspace, channel, "1700000000.000123"]).unwrap();
            cases.push(invalid);
            let mut invalid = baseline.clone();
            invalid.room.thread = Some(platform::types::ThreadKey::new(
                PlatformKind::Slack, &[workspace, channel, "1700000000.000001"]).unwrap());
            cases.push(invalid);
            let mut invalid = baseline.clone();
            invalid.reply_to.as_mut().unwrap().id = MessageKey::new(
                PlatformKind::Slack, &[workspace, channel, "1700000000.000100"]).unwrap();
            cases.push(invalid);
        }
        let mut invalid = baseline.clone();
        invalid.room.space = Some(SpaceKey::new(PlatformKind::Slack, &["T2"]).unwrap());
        cases.push(invalid);
        let mut invalid = baseline.clone();
        invalid.reply_to.as_mut().unwrap().author = Some(
            UserKey::new(PlatformKind::Slack, &["T2", "U2"]).unwrap());
        cases.push(invalid);
        for invalid in cases {
            assert!(!valid_message_source(&invalid, &connection, PlatformKind::Slack));
        }
        let mut envelope = EventEnvelope {
            connection,
            event_id: platform::types::EventKey::new(PlatformKind::Slack, &["T1", "E1"]).unwrap(),
            occurred_at_unix_ms: 1,
            event: ChatEvent::MessageCreated(baseline),
        };
        assert!(valid_envelope_source(&envelope, PlatformKind::Slack));
        envelope.event_id = platform::types::EventKey::new(PlatformKind::Slack, &["T2", "E1"]).unwrap();
        assert!(!valid_envelope_source(&envelope, PlatformKind::Slack));
    }

    #[test]
    fn same_platform_wrong_telegram_chat_message_and_topic_are_rejected() {
        let connection = ConnectionId::new("source");
        let mut message = slack_message();
        message.id = MessageKey::new(PlatformKind::Telegram, &["-100123", "81"]).unwrap();
        message.room.id = RoomKey::new(PlatformKind::Telegram, &["-100123"]).unwrap();
        message.room.space = None;
        message.room.thread = Some(platform::types::ThreadKey::new(
            PlatformKind::Telegram, &["-100123", "42"]).unwrap());
        message.author.id = UserKey::new(PlatformKind::Telegram, &["1"]).unwrap();
        message.mentions.clear();
        message.reply_to = None;
        assert!(valid_message_source(&message, &connection, PlatformKind::Telegram));
        let mut wrong_message = message.clone();
        wrong_message.id = MessageKey::new(PlatformKind::Telegram, &["-100124", "81"]).unwrap();
        assert!(!valid_message_source(&wrong_message, &connection, PlatformKind::Telegram));
        let mut wrong_topic = message.clone();
        wrong_topic.room.thread = Some(platform::types::ThreadKey::new(
            PlatformKind::Telegram, &["-100124", "42"]).unwrap());
        assert!(!valid_message_source(&wrong_topic, &connection, PlatformKind::Telegram));
        message.reply_to = Some(platform::types::MessageRef {
            id: MessageKey::new(PlatformKind::Telegram, &["-100124", "80"]).unwrap(),
            room: message.room.id.clone(),
            author: None,
        });
        assert!(!valid_message_source(&message, &connection, PlatformKind::Telegram));
    }

    #[test]
    fn matrix_federated_speakers_are_not_restricted_to_room_homeserver() {
        let mut message = slack_message();
        message.id = MessageKey::new(PlatformKind::Matrix, &["$event:room.example"]).unwrap();
        message.room.id = RoomKey::new(PlatformKind::Matrix, &["!room:room.example"]).unwrap();
        message.room.space = None;
        message.room.thread = None;
        message.author.id = UserKey::new(PlatformKind::Matrix, &["@alice:user.example"]).unwrap();
        message.mentions = vec![UserKey::new(PlatformKind::Matrix, &["@bob:third.example"]).unwrap()];
        message.reply_to = None;
        assert!(valid_message_source(&message, &ConnectionId::new("source"), PlatformKind::Matrix));
    }
}

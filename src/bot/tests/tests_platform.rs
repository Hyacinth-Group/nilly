    fn observed_person(id: UserKey, name: &str) -> UserRef {
        UserRef { id, display_name: name.into(), username: None,
            automation: platform::types::Automation::Human,
            stability: platform::types::IdentityStability::Persistent }
    }

    fn route_fixture() -> Arc<Bot> {
        Arc::new(Bot {
            state: Mutex::new(State::default()),
            adapters: Mutex::new(platform::ConnectionRegistry::default()),
            cycles_started: AtomicBool::new(false),
            announcement_sent: AtomicBool::new(false),
            http: reqwest::Client::new(), api_url: "https://invalid.example".into(), key: String::new(),
            news_channel: None, firecrawl: None, guild_id: None, allowed_channels: None,
            debug_channel: None, image_analysis: false, stealth: false,
            reasoning_mandatory_models: Mutex::new(HashSet::new()),
        })
    }

    #[test]
    fn conversations_isolate_threads_and_platforms_with_identical_native_ids() {
        let plain = RoomRef::from_key(RoomKey::discord(87201)).conversation();
        let topic = ConversationKey { room: plain.room.clone(), thread: Some(platform::types::ThreadKey::discord(1)) };
        let other = RoomRef::from_key("telegram:room:87201".parse().unwrap()).conversation();
        let mut state = State::default();
        for route in [&plain, &topic, &other] {
            start_chat(&mut state, route, None);
            state.busy.insert(route.clone());
            state.pending_mentions.push((route.clone(), "message".into(), observed_person(UserKey::discord(87202), "Emin")));
        }
        state.chats.get_mut(&topic).unwrap().incoming = 4;
        assert!(!new_message_arrived(&state, &plain, 0));
        assert!(new_message_arrived(&state, &topic, 0));
        assert!(!new_message_arrived(&state, &other, 0));
        end_chat(&mut state, &topic);
        assert!(state.chats.contains_key(&plain));
        assert!(state.chats.contains_key(&other));
        assert_eq!(state.pending_mentions.len(), 3);
    }

    #[test]
    fn queued_observations_do_not_rebind_identical_names_or_changed_names() {
        let mut state = State::default();
        let discord = observed_person(UserKey::discord(42), "Ada");
        let telegram = observed_person("telegram:user:42".parse().unwrap(), "Ada");
        let original = observe_user(&mut state, &discord);
        assert_eq!(original, "Ada");
        remember(&mut state, &original, "first");
        let snapshot = state.recent_users.clone();
        let other = observe_user(&mut state, &telegram);
        assert_eq!(other, "Ada [telegram:user:42]");
        assert_eq!(state.recent_users["ada"].id, discord.id);
        assert_eq!(state.recent_users[&other.to_lowercase()].id, telegram.id);
        let renamed = observed_person(discord.id.clone(), "New Ada");
        let renamed_label = observe_user(&mut state, &renamed);
        assert_eq!(state.recent_users[&renamed_label.to_lowercase()].id, discord.id);
        assert_eq!(snapshot["ada"].display_name, "Ada");
        assert_eq!(snapshot["ada"].id, discord.id);
        let mut first = user("Ada: first");
        first.observed_user = Some(discord.clone());
        let mut second = user(format!("{other}: second"));
        second.observed_user = Some(telegram.clone());
        let captured = observed_users(&[first, second, user("Old Ada: unknown")]);
        assert_eq!(captured["ada"].id, discord.id);
        assert_eq!(captured[&other.to_lowercase()].id, telegram.id);
        assert!(!captured.contains_key("old ada"));
    }

    #[test]
    fn history_seed_attributes_only_companion_observations_not_name_matches() {
        let route = RoomRef::from_key(RoomKey::discord(87203)).conversation();
        let person = observed_person(UserKey::discord(87204), "Ada");
        let mut state = State { bot_name: "Bot".into(), ..State::default() };
        observe_user(&mut state, &person);
        state.channel_history.insert(route.clone(), VecDeque::from(["Ada: legacy".into(), "Ada: observed".into()]));
        state.channel_observations.insert(route.clone(), VecDeque::from([None, Some(person.clone())]));
        let chat = start_chat(&mut state, &route, None);
        assert!(chat.history[0].observed_user.is_none());
        assert_eq!(chat.history[1].observed_user.as_ref().unwrap().id, person.id);
        assert_eq!(observed_users(&chat.history)["ada"].id, person.id);
    }

    #[test]
    fn companion_provenance_survives_restart_without_rewriting_legacy_text() {
        memory::test::ensure_test_db();
        let route = ConversationKey { room: RoomKey::discord(87205), thread: Some(platform::types::ThreadKey::discord(87206)) };
        let person = observed_person(UserKey::discord(87207), "Ada");
        let mut state = State::default();
        channel_note(&mut state, &route, "Ada: legacy".into());
        channel_observe(&mut state, &route, "Ada: observed".into(), &person);
        assert_eq!(memory::read(&platform::keys::room_record(&route)), "Ada: legacy\nAda: observed");
        let metadata = memory::load_channel_observations().into_iter().find(|(key, _)| key == &route).unwrap().1;
        assert!(metadata[0].is_none());
        assert_eq!(metadata[1].as_ref().unwrap().id, person.id);
    }

    #[test]
    fn core_suppresses_self_and_known_automation_but_keeps_unknown_honest() {
        let bot = route_fixture();
        let connection = ConnectionId::new("test-self");
        let account = observed_person(UserKey::discord(87208), "Bot");
        bot.state().accounts.insert(connection.clone(), account.clone());
        assert!(!bot.accepts_author(&connection, &account));
        let mut person = observed_person(UserKey::discord(87209), "Ada");
        assert!(bot.accepts_author(&connection, &person));
        person.automation = platform::types::Automation::Unknown;
        assert!(bot.accepts_author(&connection, &person));
        person.automation = platform::types::Automation::Bot;
        assert!(!bot.accepts_author(&connection, &person));
        person.automation = platform::types::Automation::Integration;
        assert!(!bot.accepts_author(&connection, &person));
    }

    #[tokio::test]
    async fn bound_ingress_analysis_off_self_and_disconnected_routes() {
        let bot = route_fixture();
        bot.put_to_sleep(1);
        let connection = ConnectionId::new("test-ingress");
        let account = observed_person(UserKey::discord(87210), "Bot");
        let room = RoomRef::from_key(RoomKey::discord(87211));
        let adapter = platform::discord::test_adapter(connection.clone(), account.clone());
        {
            let mut registry = bot.adapters.lock().unwrap_or_else(|error| error.into_inner());
            registry.register(adapter);
            registry.set_room_policy(&connection, platform::registry::RoomPolicy::Approved(HashSet::from([room.id.clone()]))).unwrap();
            registry.bind(&connection, room.clone()).unwrap();
        }
        bot.state().accounts.insert(connection.clone(), account.clone());
        let mut message = IncomingMessage {
            id: MessageKey::discord(87212), room: room.clone(),
            author: observed_person(UserKey::discord(87213), "Ada"), sent_at_unix_ms: 0,
            text: "look".into(), mentions: vec![account.id.clone()], reply_to: None,
            attachments: vec![platform::types::AttachmentRef { kind: MediaKind::Image, mime: Some("image/png".into()), size: None,
                source: MediaSource::Native { connection: connection.clone(), id: "private-token-never-record".into() } }],
        };
        bot.message_created(&connection, message.clone()).await;
        let (adapter, _) = bot.transport(&room.conversation()).unwrap();
        let platform::Adapter::Discord(adapter) = adapter.as_ref() else {
            panic!("expected Discord fixture adapter");
        };
        assert_eq!(adapter.media_resolution_count(), 0);
        {
            let state = bot.state();
            assert_eq!(state.recent_messages.back().unwrap(), "Ada: look");
            assert_eq!(state.pending_mentions[0].2.id, message.author.id);
            assert!(!recent_messages(&state, 10).contains("private-token"));
        }
        message.author = account;
        bot.message_created(&connection, message.clone()).await;
        assert_eq!(bot.state().growth.messages, 1);
        bot.adapters.lock().unwrap_or_else(|error| error.into_inner()).set_status(&connection, platform::outgoing::ConnectionStatus::Disconnected).unwrap();
        assert!(bot.transport(&room.conversation()).is_err());
        message.author = observed_person(UserKey::discord(87213), "Ada");
        bot.message_created(&connection, message).await;
        assert_eq!(bot.state().growth.messages, 1);
    }

    #[test]
    fn transport_snapshots_actual_bound_topic_and_never_fabricates_unbound_rooms() {
        let bot = route_fixture();
        let connection = ConnectionId::new("test-topics");
        let account = observed_person(UserKey::discord(87220), "Bot");
        let mut topic = RoomRef::from_key(RoomKey::discord(87221));
        topic.space = Some(SpaceKey::discord(87222));
        topic.name = Some("Actual Native Topic".into());
        topic.thread = Some(platform::types::ThreadKey::discord(87223));
        {
            let mut registry = bot.adapters.lock().unwrap_or_else(|error| error.into_inner());
            registry.register(platform::discord::test_adapter(connection.clone(), account));
            registry.set_room_policy(&connection, platform::registry::RoomPolicy::Approved(HashSet::from([topic.id.clone()]))).unwrap();
            registry.bind_discovered(&connection, topic.clone()).unwrap();
        }
        let (adapter, actual) = bot.transport(&topic.conversation()).unwrap();
        assert_eq!(adapter.connection_id(), &connection);
        assert_eq!(actual.thread, topic.thread);
        assert_eq!(actual.space, topic.space);
        assert_eq!(actual.name, topic.name);
        assert!(bot.transport(&RoomRef::from_key(topic.id.clone()).conversation()).is_err());
        let unapproved = RoomRef::from_key(RoomKey::discord(87224));
        assert!(!bot.accepts_room(&connection, &unapproved));
        assert!(bot.transport(&unapproved.conversation()).is_err());
        assert!(!bot.accepts_room(&ConnectionId::new("other-connection"), &topic));
    }

    #[test]
    fn prepared_instruction_and_inserted_values_are_not_interpreted_as_prompt_tokens() {
        let context = platform::prompt_context::PromptContext::for_platform(platform::types::PlatformKind::Telegram);
        let raw = prompts::current().critic;
        let literal = "{ad} {platform_en_ai_identity} {platform_tr_ai_identity}";
        let prepared = context.render_with(raw, &[("{ad}", literal), ("{mevcut}", literal)]);
        assert!(prepared.contains(literal));
        let state = State { bot_name: literal.into(), ..State::default() };
        let instruction = format!("{prepared}\n\nresearch: {literal}");
        let (fixed, variable) = system_text_with(&state, &instruction, literal, &context);
        assert!(fixed.contains(literal));
        assert!(variable.contains(&instruction));
        assert!(variable.contains(literal));
    }

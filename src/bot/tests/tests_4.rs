    use crate::platform::discord::events::{discord_message_input, display_name, reaction_label};

    /// Verifies `parse_reply`/`number_prefix` strip a list-item number prefix only across a real multi-line list, never on a lone line that's actually a Turkish ordinal ("3. sınıftayım").
    #[test]
    fn number_prefix_only_stripped_in_real_list() {
        // multiple numbered lines = the model wrote a list, the prefix is stripped
        let reply = parse_reply("1. şunu yap\n2) sonra bunu", OutputLimits::DISCORD);
        assert_eq!(reply.lines, vec!["şunu yap", "sonra bunu"]);
        // a number on a single line is a Turkish ordinal, its meaning must not be lost
        assert_eq!(
            parse_reply("3. sınıftayım", OutputLimits::DISCORD).lines,
            vec!["3. sınıftayım"]
        );
        assert_eq!(
            parse_reply("2. el araba aldım", OutputLimits::DISCORD).lines,
            vec!["2. el araba aldım"]
        );
        assert_eq!(number_prefix("12) madde"), Some("madde"));
        assert_eq!(number_prefix("3.14 sayısı"), None);
    }

    /// Verifies `parse_reply` collapses an immediately repeated line into one.
    #[test]
    fn same_line_not_sent_twice() {
        assert_eq!(parse_reply("he\nhe", OutputLimits::DISCORD).lines, vec!["he"]);
    }

    /// Verifies `parse_reply` splits a single line longer than `MESSAGE_LIMIT` into multiple lines, each within the limit.
    #[test]
    fn reply_long_line_gets_split() {
        let long_text = "a".repeat(MESSAGE_LIMIT + 100);
        let reply = parse_reply(&long_text, OutputLimits::DISCORD);
        assert_eq!(reply.lines.len(), 2);
        for line in &reply.lines {
            assert!(line.chars().count() <= MESSAGE_LIMIT);
        }
    }

    /// Verifies `Reply::protocol_text` reconstructs the original line-based protocol text (lines plus a `tepki:` line) from a parsed reply.
    #[test]
    fn protocol_text_round_trips() {
        assert_eq!(
            parse_reply("hahaha\ntepki: 💀", OutputLimits::DISCORD).protocol_text(),
            "hahaha\ntepki: 💀"
        );
        assert_eq!(
            parse_reply("tepki: 💀", OutputLimits::DISCORD).protocol_text(),
            "tepki: 💀"
        );
        assert_eq!(
            parse_reply("bir\niki", OutputLimits::DISCORD).protocol_text(),
            "bir\niki"
        );
    }

    /// Verifies `stream_slice` holds back an incomplete short trailing line while streaming (it could still turn into a reaction marker), but reveals everything once the stream is done.
    #[test]
    fn stream_slice_holds_back_half_line() {
        // while streaming, an incomplete short line stays hidden ("tep" could become "tepki: 💀")
        assert_eq!(stream_slice("selam\ntep", false), "selam");
        // a long enough half-line does enter the layout
        let text = "selam\nbu satır yeterince uzun";
        assert_eq!(stream_slice(text, false), text);
        // a short single-line stream isn't shown yet
        assert_eq!(stream_slice("kısa", false), "");
        // once the stream is done, everything is visible
        assert_eq!(stream_slice("selam\ntep", true), "selam\ntep");
    }

    /// Verifies `stream_view` turns each reply line into its own message, holds back an incomplete trailing line while streaming, drops a reaction line from the message list, and emits nothing for the silence marker.
    #[test]
    fn view_turns_lines_into_messages() {
        // each line becomes its own message
        let view = stream_view(ThinkingMode::Off, "", "bir\niki", true);
        assert_eq!(view, vec!["bir", "iki"]);
        // while streaming, a half line is held back
        let view = stream_view(ThinkingMode::Off, "", "hahaha\ntep", false);
        assert_eq!(view, vec!["hahaha"]);
        // a reaction line doesn't become a message
        let view = stream_view(ThinkingMode::Off, "", "hahaha\ntepki: 💀", true);
        assert_eq!(view, vec!["hahaha"]);
        // the silence marker never enters the layout at all
        assert!(stream_view(ThinkingMode::Off, "", "-", true).is_empty());
    }

    /// Verifies `too_many_questions` counts only question-shaped bot lines (not reactions) against the channel's recent history, and returns false for a channel with no history.
    #[test]
    fn question_cap_fills() {
        let channel = RoomKey::discord(3);
        let mut state = State {
            bot_name: "kaju".into(),
            ..State::default()
        };
        let full: VecDeque<String> = [
            "emin: naber",
            "kaju: iyidir sen?",
            "emin: iyi",
            "kaju: ne yapıyosun?",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        state.channel_history.insert(channel.clone(), full);
        assert!(too_many_questions(&state, &channel));
        // reaction lines don't count; plain talk mixed in keeps the cap from filling
        let sparse: VecDeque<String> = ["kaju: iyidir sen?", "kaju: tepki: 💀", "kaju: aynen"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        state.channel_history.insert(channel.clone(), sparse);
        assert!(!too_many_questions(&state, &channel));
        // a channel with no history
        assert!(!too_many_questions(&state, &RoomKey::discord(99)));
    }

    /// Verifies `message_json` emits plain string content for a text-only message, a text+image_url array when an image is attached, and that an assistant message never carries an image.
    #[test]
    fn message_json_becomes_array_with_image() {
        // no image: plain text content
        let json = message_json(&user("emin: selam"));
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "emin: selam");
        // with an image: text + image_url parts (same shape the image commenter uses)
        let json = message_json(&user_with_image(
            "emin: [resim] şuna bak",
            "https://cdn.discordapp.com/a.png",
        ));
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][0]["text"], "emin: [resim] şuna bak");
        assert_eq!(json["content"][1]["type"], "image_url");
        assert_eq!(
            json["content"][1]["image_url"]["url"],
            "https://cdn.discordapp.com/a.png"
        );
        // an assistant message never carries an image
        assert_eq!(message_json(&assistant("he"))["content"], "he");
    }

    /// Verifies `strip_name` borrows from its input (`&str` in, `&str` out) rather than allocating a new string.
    #[test]
    fn strip_name_returns_slice() {
        // strip_name takes &str, returns &str; no full-text clone needed
        let text = String::from("cicikus: merhaba dünya");
        let slice: &str = strip_name(&text, "cicikus");
        assert_eq!(slice, "merhaba dünya");
    }

    /// Verifies `reaction_label` passes a unicode emoji through as-is, wraps a named custom
    /// emoji in colons instead of Discord's raw `<:name:id>` mention form, and falls back to
    /// a placeholder when even the name is missing.
    #[test]
    fn reaction_label_formats_emoji() {
        assert_eq!(reaction_label(&ReactionType::Unicode("💀".into())), "💀");
        assert_eq!(
            reaction_label(&ReactionType::Custom {
                animated: false,
                id: EmojiId::new(1),
                name: Some("kekw".into()),
            }),
            ":kekw:"
        );
        assert_eq!(
            reaction_label(&ReactionType::Custom {
                animated: false,
                id: EmojiId::new(1),
                name: None,
            }),
            "bilinmeyen emoji"
        );
    }

    fn discord_event_fixture(content: &str) -> Message {
        let mut msg = Message::default();
        msg.id = MessageId::new(81);
        msg.channel_id = ChannelId::new(7);
        msg.guild_id = Some(GuildId::new(9));
        msg.author.id = UserId::new(42);
        msg.author.name = "account".into();
        msg.content = content.into();
        msg
    }

    fn discord_attachment_fixture(
        filename: &str,
        content_type: Option<&str>,
        url: &str,
    ) -> Attachment {
        serde_json::from_value(serde_json::json!({
            "id": "1",
            "filename": filename,
            "content_type": content_type,
            "url": url,
            "proxy_url": "https://media.discordapp.net/proxy.png",
            "size": 123,
            "width": 100,
            "height": 100
        }))
        .unwrap()
    }

    /// Exercises the live decoder's author/webhook/guild exclusions independently,
    /// including a webhook whose author is not marked as a bot.
    #[test]
    fn discord_input_excludes_bots_webhooks_and_missing_guilds() {
        let cache = Arc::new(serenity::cache::Cache::new());
        for bot in [false, true] {
            for webhook in [false, true] {
                for guild in [false, true] {
                    let mut msg = discord_event_fixture("hello");
                    msg.author.bot = bot;
                    msg.webhook_id = webhook.then(|| WebhookId::new(55));
                    msg.guild_id = guild.then(|| GuildId::new(9));
                    let input = discord_message_input(&msg, &cache, None, None, false);
                    if !bot && !webhook && guild {
                        assert_eq!(input, Some(("hello".into(), None)));
                    } else {
                        assert_eq!(input, None, "bot={bot}, webhook={webhook}, guild={guild}");
                    }
                }
            }
        }

        // Application metadata is not a webhook author, and `system` isn't `bot`.
        let mut msg = discord_event_fixture("hello");
        msg.application_id = Some(ApplicationId::new(55));
        msg.author.system = true;
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some(("hello".into(), None))
        );
    }

    /// Pins the optional GUILD_ID/CHANNELS restrictions without changing defaults.
    #[test]
    fn discord_input_respects_configured_guild_and_channel_scope() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let msg = discord_event_fixture("hello");
        let allowed = HashSet::from([msg.channel_id]);
        let other = HashSet::from([ChannelId::new(8)]);
        let empty = HashSet::new();
        assert_eq!(
            discord_message_input(&msg, &cache, msg.guild_id, Some(&allowed), false),
            Some(("hello".into(), None))
        );
        assert_eq!(
            discord_message_input(&msg, &cache, Some(GuildId::new(10)), None, false),
            None
        );
        assert_eq!(
            discord_message_input(&msg, &cache, None, Some(&other), false),
            None
        );
        assert_eq!(
            discord_message_input(&msg, &cache, None, Some(&empty), false),
            None
        );
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some(("hello".into(), None))
        );
    }

    /// Captures Message::content_safe, not Serenity's separate utils decoder:
    /// usernames (not global names), roles, cancelled mass mentions, untouched channels.
    #[test]
    fn discord_input_uses_exact_serenity_message_safe_content() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let mut role = Role::default();
        role.id = RoleId::new(5);
        role.name = "friends".into();
        let mut guild = Guild::default();
        guild.id = GuildId::new(9);
        guild.roles.insert(role.id, role);
        let mut event: GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).unwrap()).unwrap();
        cache.update(&mut event);

        let mut msg = discord_event_fixture(
            "  <@42> <@!42> <@!43> <@999> <@&5> <@&6> <#7> @everyone @here\n",
        );
        let mut legacy = msg.author.clone();
        legacy.global_name = Some("Visible Name".into());
        legacy.discriminator = std::num::NonZeroU16::new(7);
        let mut modern = User::default();
        modern.id = UserId::new(43);
        modern.name = "modern".into();
        modern.global_name = Some("Other Display".into());
        msg.mentions = vec![legacy, modern];
        msg.mention_roles = vec![RoleId::new(5), RoleId::new(6)];
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some((
                "  @account#0007 <@!42> @modern <@999> @friends @deleted-role <#7> @\u{200B}everyone @\u{200B}here\n".into(),
                None
            ))
        );
    }

    /// MIME matching is ordered, lowercase and prefix-only; neither filenames nor
    /// dimensions determine eligibility, and the original URL beats the proxy URL.
    #[test]
    fn discord_input_selects_first_matching_mime_image() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let mut msg = discord_event_fixture("look");
        msg.attachments = vec![
            discord_attachment_fixture("missing.png", None, "https://cdn.example/missing"),
            discord_attachment_fixture(
                "uppercase.png",
                Some("Image/png"),
                "https://cdn.example/uppercase",
            ),
            discord_attachment_fixture(
                "document.png",
                Some("application/octet-stream"),
                "https://cdn.example/document",
            ),
            discord_attachment_fixture(
                "first.txt",
                Some("image/svg+xml"),
                "https://cdn.discordapp.com/first.svg",
            ),
            discord_attachment_fixture(
                "second.png",
                Some("image/png"),
                "https://cdn.discordapp.com/second.png",
            ),
        ];
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, true),
            Some((
                "[resim] look".into(),
                Some("https://cdn.discordapp.com/first.svg".into())
            ))
        );
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some(("look".into(), None))
        );
    }

    /// The current starts_with check does not validate the MIME subtype or inspect
    /// the file extension, dimensions, embeds, or URL when the MIME is missing.
    #[test]
    fn discord_input_image_eligibility_is_only_lowercase_mime_prefix() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let url = "https://cdn.discordapp.com/image";
        for mime in [Some("image/"), Some("image/not-a-real-subtype")] {
            let mut msg = discord_event_fixture("");
            let mut attachment = discord_attachment_fixture("file.txt", mime, url);
            attachment.width = None;
            attachment.height = None;
            msg.attachments.push(attachment);
            assert_eq!(
                discord_message_input(&msg, &cache, None, None, true),
                Some(("[resim attı]".into(), Some(url.into())))
            );
        }
        for mime in [None, Some("Image/png"), Some(" image/png"), Some("video/png")] {
            let mut msg = discord_event_fixture("");
            msg.attachments.push(discord_attachment_fixture("image.png", mime, url));
            assert_eq!(discord_message_input(&msg, &cache, None, None, true), None);
        }

        let mut msg = discord_event_fixture("");
        msg.embeds.push(
            serde_json::from_value(serde_json::json!({
                "type": "image",
                "image": {"url": url}
            }))
            .unwrap(),
        );
        assert_eq!(discord_message_input(&msg, &cache, None, None, true), None);
    }

    /// IMAGE_ANALYSIS off treats image-only input as empty and never adds a marker;
    /// trimming is just an emptiness check, not a rewrite of ordinary message text.
    #[test]
    fn discord_input_image_markers_and_analysis_off_match_existing_behavior() {
        let cache = Arc::new(serenity::cache::Cache::new());
        let url = "https://cdn.discordapp.com/image.png";
        for text in ["", " \n\t", "\u{2003}"] {
            let mut msg = discord_event_fixture(text);
            assert_eq!(discord_message_input(&msg, &cache, None, None, true), None);
            assert_eq!(discord_message_input(&msg, &cache, None, None, false), None);
            msg.attachments.push(discord_attachment_fixture("image.png", Some("image/png"), url));
            assert_eq!(
                discord_message_input(&msg, &cache, None, None, true),
                Some(("[resim attı]".into(), Some(url.into())))
            );
            assert_eq!(discord_message_input(&msg, &cache, None, None, false), None);
        }

        let mut msg = discord_event_fixture("  look\n ");
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some(("  look\n ".into(), None))
        );
        msg.attachments.push(discord_attachment_fixture("image.png", Some("image/png"), url));
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, true),
            Some(("[resim]   look\n ".into(), Some(url.into())))
        );
        assert_eq!(
            discord_message_input(&msg, &cache, None, None, false),
            Some(("  look\n ".into(), None))
        );
    }

    /// Display names prefer Some(global_name) even when empty; guild nicknames are
    /// not consulted and account names are returned verbatim when global_name is None.
    #[test]
    fn discord_display_name_fallback_is_option_based() {
        let mut msg = discord_event_fixture("hello");
        msg.author.name = " account ".into();
        msg.author.member = Some(Box::new(
            serde_json::from_value(serde_json::json!({
                "nick": "Guild Nickname",
                "roles": []
            }))
            .unwrap(),
        ));
        assert_eq!(display_name(&msg.author), " account ");
        msg.author.global_name = Some(" Global Name ".into());
        assert_eq!(display_name(&msg.author), " Global Name ");
        msg.author.global_name = Some(String::new());
        assert_eq!(display_name(&msg.author), "");
        msg.author.global_name = None;
        assert_eq!(display_name(&msg.author), " account ");
    }

    /// Animation and IDs don't change custom emoji labels; Some("") is not the
    /// unnamed fallback, and composed Unicode is passed through without normalization.
    #[test]
    fn discord_reaction_labels_preserve_custom_name_and_unicode_quirks() {
        assert_eq!(
            reaction_label(&ReactionType::Custom {
                animated: true,
                id: EmojiId::new(99),
                name: Some("party_blob".into()),
            }),
            ":party_blob:"
        );
        assert_eq!(
            reaction_label(&ReactionType::Custom {
                animated: true,
                id: EmojiId::new(100),
                name: None,
            }),
            "bilinmeyen emoji"
        );
        assert_eq!(
            reaction_label(&ReactionType::Custom {
                animated: false,
                id: EmojiId::new(101),
                name: Some(String::new()),
            }),
            "::"
        );
        assert_eq!(reaction_label(&ReactionType::Unicode("👩🏽‍💻".into())), "👩🏽‍💻");
    }

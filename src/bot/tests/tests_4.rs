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
        let channel = ConversationKey { room: RoomKey::discord(3), thread: None };
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
        assert!(!too_many_questions(&state, &ConversationKey { room: RoomKey::discord(99), thread: None }));
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


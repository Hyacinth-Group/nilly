    /// Verifies `StreamReader::next` against a real SSE stream from a local TCP server, including a `[DONE]` terminator, confirming reasoning and content accumulate correctly across chunks.
    #[tokio::test]
    async fn stream_reader_parses_sse() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut buf = [0u8; 512];
            while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = conn.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&buf[..n]);
            }
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning\":\"önce düşün\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Güne\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"ş bugün güzel\"}}]}\n\n",
                "data: [DONE]\n\n",
            );
            let response_text = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            conn.write_all(response_text.as_bytes()).unwrap();
            conn.flush().unwrap();
        });
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/"))
            .json(&serde_json::json!({"stream": true}))
            .send()
            .await
            .unwrap();
        let mut reader = StreamReader {
            response: resp,
            buffer: Vec::new(),
            queue: Vec::new(),
            usage: Usage::default(),
            category: "test",
            done: false,
            finished: false,
        };
        let mut text = String::new();
        let mut thought = String::new();
        while let Some(chunk) = reader.next().await.unwrap() {
            text.push_str(&chunk.text);
            thought.push_str(&chunk.thought);
        }
        assert_eq!(thought, "önce düşün");
        assert_eq!(text, "Güneş bugün güzel");
    }

    /// Verifies `strip_name` removes a `"name: "` prefix case-insensitively, and leaves text without that prefix untouched.
    #[test]
    fn strip_name_takes_prefix() {
        assert_eq!(strip_name("cicikus: selam", "cicikus"), "selam");
        // case-insensitive
        assert_eq!(strip_name("Cicikus: selam", "cicikus"), "selam");
        // no prefix: text is left unchanged
        assert_eq!(strip_name("selam", "cicikus"), "selam");
    }

    /// Verifies `strip_name` doesn't panic on Turkish casing edge cases (`İ`→`i̇` changes the byte length).
    #[test]
    fn strip_name_doesnt_panic_on_turkish_names() {
        // İ→i̇ lowercasing changes the byte count; a byte slice here would have panicked
        assert_eq!(strip_name("Çöp: selam", "çöp"), "selam");
        assert_eq!(strip_name("İsim: merhaba", "İsim"), "merhaba");
        assert_eq!(strip_name("ŞAHİN: tamam", "şahin"), "tamam");
    }

    /// Verifies `strip_name` strips a matching pair of surrounding quotes but leaves a lone or unclosed quote alone.
    #[test]
    fn strip_name_strips_quotes() {
        assert_eq!(strip_name("\"selam\"", "bot"), "selam");
        assert_eq!(strip_name("\"", "bot"), "\""); // a lone quote is left alone
        assert_eq!(strip_name("\"selam", "bot"), "\"selam"); // unclosed: don't touch it
    }

    /// Verifies `strip_name` handles a name prefix and surrounding quotes together.
    #[test]
    fn strip_name_combined_pattern() {
        assert_eq!(strip_name("bot: \"selam dünya\"", "bot"), "selam dünya");
    }

    /// Verifies `parse_reply` splits a multi-line reply into `lines`, with no reaction/silence flags set, and that empty input parses to an empty reply.
    #[test]
    fn reply_splits_into_lines() {
        let reply = parse_reply("ilk satır\n\nikinci satır");
        assert_eq!(reply.lines, vec!["ilk satır", "ikinci satır"]);
        assert!(reply.reaction.is_none() && !reply.silent);
        assert!(parse_reply("").is_empty());
    }

    /// Verifies `parse_reply`'s `tepki:` reaction extraction: case/spacing variants, an unparseable custom-emoji form, first-reaction-wins on multiple `tepki:` lines, and a plain line with a colon not being mistaken for one.
    #[test]
    fn reply_extracts_reaction() {
        assert_eq!(parse_reply("tepki: 💀").reaction.as_deref(), Some("💀"));
        // uppercase and a space before the colon are also recognized
        assert_eq!(parse_reply("Tepki : 💀").reaction.as_deref(), Some("💀"));
        // anything after the emoji is dropped, the line doesn't go out as a message
        let reply = parse_reply("tepki: 😂 aynen");
        assert_eq!(reply.reaction.as_deref(), Some("😂"));
        assert!(reply.lines.is_empty());
        // a custom emoji form can't be parsed, but the line still isn't sent as a message
        let reply = parse_reply("tepki: :kekw:");
        assert!(reply.reaction.is_none() && reply.lines.is_empty());
        // a reaction can come along with a line of text; the first reaction wins
        let reply = parse_reply("hahaha\ntepki: 💀\ntepki: 😂");
        assert_eq!(reply.lines, vec!["hahaha"]);
        assert_eq!(reply.reaction.as_deref(), Some("💀"));
        // a plain line with a colon in it isn't mistaken for a reaction
        assert_eq!(parse_reply("saat 3: gidiyoruz").lines.len(), 1);
        // "no text, but let's still react": the silence marker doesn't drop the reaction
        let reply = parse_reply("tepki: 💀\n-");
        assert!(reply.silent && reply.lines.is_empty());
        assert_eq!(reply.reaction.as_deref(), Some("💀"));
    }

    /// Verifies `parse_reply` rejects typographic marks as reactions (Discord's API rejects them as invalid emoji) while still accepting real emoji, including inside quotes or with a variation selector.
    #[test]
    fn reaction_rejects_non_emoji() {
        // typographic marks aren't emoji: sending one to Discord as a reaction got the request rejected with a 400
        for line in ["tepki: —", "tepki: …", "tepki: →", "tepki: ¯\\_(ツ)_/¯"] {
            assert!(
                parse_reply(line).reaction.is_none(),
                "{line} counted as emoji"
            );
        }
        // an emoji inside quotes is still found, the quote mark doesn't confuse the sequence parser
        assert_eq!(parse_reply("tepki: “👍”").reaction.as_deref(), Some("👍"));
        // an emoji with a variation selector, from the symbols block, is accepted
        assert_eq!(parse_reply("tepki: ⭐").reaction.as_deref(), Some("⭐"));
    }

    /// Verifies `transcript` prefixes every line of a multi-line bot reply with the bot's name, not just the first.
    #[test]
    fn transcript_prefixes_every_bot_line() {
        // a bot reply can span several lines: the later lines belong to the bot too, the
        // critic shouldn't attribute them to a person
        let history = vec![user("emin: naber"), assistant("iyidir\ntepki: 💀")];
        assert_eq!(
            transcript(&history, "kaju"),
            "emin: naber\nkaju: iyidir\nkaju: tepki: 💀"
        );
    }

    /// Verifies `start_chat` doesn't duplicate an opening message that's already present in the channel history.
    #[test]
    fn opening_not_seeded_twice() {
        let channel = RoomKey::discord(7);
        let mut state = State {
            bot_name: "kaju".into(),
            ..State::default()
        };
        // the opening went out line by line, with a link message mixed in (built without touching disk)
        let hist: VecDeque<String> = ["emin: selam", "kaju: bir", "kaju: iki", "kaju: https://a.b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        state.channel_history.insert(channel.clone(), hist);
        let chat = start_chat(&mut state, &channel, Some("bir\niki".to_string()));
        let contents: Vec<&str> = chat.history.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(contents, vec!["emin: selam", "https://a.b", "bir\niki"]);
    }

    /// Verifies `parse_reply` recognizes the silence marker (`-` and its quoted/bracketed variants) and doesn't mistake unrelated text for it.
    #[test]
    fn reply_silence_marker() {
        let reply = parse_reply("-");
        assert!(reply.silent && reply.lines.is_empty() && !reply.is_empty());
        assert!(parse_reply("\"-\"").silent);
        assert!(parse_reply("'-'").silent);
        assert!(parse_reply("[sus]").silent);
        assert!(parse_reply("(sus)").silent);
        assert!(!parse_reply("yok artık").silent);
    }

    /// Verifies `parse_reply` caps the number of lines at `BURST_LIMIT`.
    #[test]
    fn reply_burst_limit() {
        let reply = parse_reply("bir\niki\nüç\ndört\nbeş");
        assert_eq!(reply.lines.len(), BURST_LIMIT);
        assert_eq!(reply.lines.last().unwrap(), "dört");
    }

    /// Verifies `parse_reply` drops a leftover text scrap but keeps short natural replies like "he"/"yok"/"la".
    #[test]
    fn reply_drops_scraps_keeps_short_lines() {
        // a leftover scrap of the previous message is dropped
        assert!(parse_reply("'cım").lines.is_empty());
        // short lines are no longer dropped: "he", "yok", "la" are natural replies
        assert_eq!(parse_reply("he").lines, vec!["he"]);
        assert_eq!(parse_reply("yok\nla").lines, vec!["yok", "la"]);
    }

    /// Verifies `clean_slop` strips list-marker/markdown-emphasis prefixes while leaving code spans and real numbers untouched, and that `parse_reply` applies the same cleanup.
    #[test]
    fn slop_prefixes_stripped() {
        assert_eq!(clean_slop("- madde"), "madde");
        assert_eq!(clean_slop("* madde"), "madde");
        assert_eq!(clean_slop("• madde"), "madde");
        assert_eq!(clean_slop("**kalın** laf"), "kalın laf");
        assert_eq!(clean_slop("__altı__ çizili"), "altı çizili");
        // backtick is left alone, a code fragment carries information — the INSIDE is preserved too
        assert_eq!(clean_slop("`kod` çalışmıyor"), "`kod` çalışmıyor");
        assert_eq!(
            clean_slop("`__init__` fonksiyonu"),
            "`__init__` fonksiyonu"
        );
        // a real number that looks like a list marker isn't touched
        assert_eq!(clean_slop("3.14 sayısı"), "3.14 sayısı");
        // parsing applies the same cleanup
        assert_eq!(parse_reply("- bir\n- iki").lines, vec!["bir", "iki"]);
    }


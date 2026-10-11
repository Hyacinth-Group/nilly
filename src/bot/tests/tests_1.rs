
    /// Verifies `new_message_arrived` compares against the chat's `incoming` counter to detect a newer message.
    #[test]
    fn new_message_invalidates_old_reply() {
        let channel = RoomKey::discord(7);
        let mut state = State::default();
        state.chats.insert(
            channel.clone(),
            Chat {
                incoming: 3,
                ..Chat::default()
            },
        );
        assert!(new_message_arrived(&state, &channel, 2));
        assert!(!new_message_arrived(&state, &channel, 3));
    }

    /// Verifies `split` prefers cutting at sentence boundaries and every part stays within the limit.
    #[test]
    fn split_sentence_boundary() {
        let text = "birinci cümle burada. ikinci cümle şurada. üçüncüsü de ötede.";
        let parts = split(text, 30);
        assert!(parts.len() >= 2);
        for part in &parts {
            assert!(part.chars().count() <= 30);
        }
        let joined: String = parts.join(" ");
        assert_eq!(joined.replace(' ', ""), text.replace(' ', ""));
    }

    /// Verifies `split` falls back to cutting at a space when there's no punctuation to break on.
    #[test]
    fn split_falls_back_to_space() {
        // no punctuation: cuts at a space
        let text = "aaaa bbbb cccc dddd eeee";
        let result = split(text, 12);
        assert_eq!(result, vec!["aaaa bbbb", "cccc dddd", "eeee"]);
    }

    /// Verifies `split` hard-cuts at the exact limit when there's no space either, dropping no characters.
    #[test]
    fn split_hard_cuts() {
        // no space at all: cuts at the exact limit, drops nothing
        let text = "a".repeat(50);
        let parts = split(&text, 20);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts.iter().map(|s| s.chars().count()).sum::<usize>(), 50);
    }

    /// Verifies `split` leaves text under the limit untouched and returns nothing for blank input.
    #[test]
    fn short_text_untouched() {
        assert_eq!(split("kısa", 100), vec!["kısa"]);
        assert_eq!(split("  ", 100), Vec::<String>::new());
    }

    /// Verifies `split`'s cut point stays character-aligned on multibyte (Turkish) text, never panicking on a byte boundary.
    #[test]
    fn split_doesnt_panic_on_multibyte_boundary() {
        // cut_point returns a byte offset; even at a multibyte character boundary the
        // slice stays character-aligned, and the parts equal the original once rejoined
        let text = "üğşçöı ğüşçöı ğüşçöı ğüşçöı ğüşçöı";
        let parts = split(text, 8);
        for part in &parts {
            assert!(part.chars().count() <= 8);
        }
        assert_eq!(parts.join(" ").replace(' ', ""), text.replace(' ', ""));
    }

    /// Verifies `spoiler` wraps text in `||...||` and escapes a literal `|` inside it.
    #[test]
    fn spoiler_escapes() {
        assert_eq!(spoiler("düşünce"), "||düşünce||");
        assert_eq!(spoiler("a|b"), "||a\\|b||");
    }

    /// Verifies `extract_sse` across the SSE chunk shapes seen in practice: content+reasoning, content-only (mistral), `reasoning_content` (OpenAI-compatible routers), the `[DONE]` marker, a final usage-only chunk, and malformed/keepalive lines that should parse to `None`.
    #[test]
    fn sse_parses() {
        let result = extract_sse(r#"data: {"choices":[{"delta":{"content":"sel","reasoning":"düş"}}]}"#)
            .unwrap();
        let chunk = result.chunk.unwrap();
        assert_eq!(chunk.text, "sel");
        assert_eq!(chunk.thought, "düş");
        assert!(result.usage.is_none());
        // no reasoning (mistral-style): content still comes through
        let chunk = extract_sse(r#"data: {"choices":[{"delta":{"content":"merhaba"}}]}"#)
            .unwrap()
            .chunk
            .unwrap();
        assert_eq!(chunk.text, "merhaba");
        assert!(chunk.thought.is_empty());
        // OpenAI-compatible routers use reasoning_content
        let chunk = extract_sse(
            r#"data: {"choices":[{"delta":{"content":"","reasoning_content":"qwen düşüncesi"}}]}"#,
        )
        .unwrap()
        .chunk
        .unwrap();
        assert_eq!(chunk.thought, "qwen düşüncesi");
        assert!(chunk.text.is_empty());
        // the [DONE] marker is caught separately
        let result = extract_sse("data: [DONE]").unwrap();
        assert!(result.done && result.chunk.is_none());
        // usage arrives on the final chunk with choices empty
        let result = extract_sse(
            r#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":13}}"#,
        )
        .unwrap();
        assert!(result.chunk.is_none());
        assert_eq!(result.usage.unwrap().prompt_tokens, 7);
        assert_eq!(result.usage.unwrap().completion_tokens, 13);
        assert!(extract_sse(": keepalive").is_none());
        assert!(extract_sse("data: bozuk json").is_none());
        assert!(extract_sse(r#"data: {"choices":[{"delta":{}}]}"#).is_none());
    }

    /// Verifies `stream_view`'s Show-mode layout (spoiler block, then code block, then reply parts, each within `MESSAGE_LIMIT`) and that Hide mode never emits the thought at all.
    #[test]
    fn view_gets_split() {
        let thought = "düşün ".repeat(700); // ~4200 chars, needs multiple blocks
        let reply = "kelime ".repeat(400); // ~2800 chars, needs multiple messages
        let view = stream_view(ThinkingMode::Show, &thought, &reply, true);
        assert!(view.len() >= 5);
        for (i, line) in view.iter().enumerate() {
            assert!(line.chars().count() <= MESSAGE_LIMIT, "part {i} too long");
        }
        // spoiler blocks first, then code blocks, reply parts last
        assert!(view[0].starts_with("||") && view[0].ends_with("||"));
        assert!(view.iter().any(|m| m.starts_with("```")));
        assert!(!view[view.len() - 1].starts_with("||"));
        // hide: the thought never enters the layout at all
        let view = stream_view(ThinkingMode::Hide, &thought, &reply, true);
        assert!(view
            .iter()
            .all(|m| !m.starts_with("||") && !m.starts_with("```")));
    }

    /// Verifies `stream_view` with an empty thought just returns the reply as a single message.
    #[test]
    fn view_without_thought() {
        let result = stream_view(ThinkingMode::Show, "", "kısa cevap", true);
        assert_eq!(result, vec!["kısa cevap"]);
    }

    /// Verifies `ThinkingMode::from_arg`'s recognized values (including synonyms) and `file_value`'s round-trip strings.
    #[test]
    fn thinking_mode_parses() {
        assert_eq!(ThinkingMode::from_arg("göster"), Some(ThinkingMode::Show));
        assert_eq!(ThinkingMode::from_arg("aç"), Some(ThinkingMode::Show));
        assert_eq!(ThinkingMode::from_arg("gizle"), Some(ThinkingMode::Hide));
        assert_eq!(ThinkingMode::from_arg("sessiz"), Some(ThinkingMode::Silent));
        assert_eq!(ThinkingMode::from_arg("kapat"), Some(ThinkingMode::Off));
        assert_eq!(ThinkingMode::from_arg("kapalı"), Some(ThinkingMode::Off));
        assert_eq!(ThinkingMode::from_arg("saçma"), None);
        assert_eq!(ThinkingMode::Show.file_value(), "goster");
        assert_eq!(ThinkingMode::Silent.file_value(), "sessiz");
    }

    /// Verifies `stream_view`'s placeholder text while the reply hasn't started yet, across all four `ThinkingMode`s.
    #[test]
    fn view_placeholder_while_thinking() {
        // show: plain placeholder
        let result = stream_view(ThinkingMode::Show, "hmm düşünüyorum", "", true);
        assert_eq!(result, vec!["Düşünüyorum..."]);
        // hide: live word counter
        let result = stream_view(ThinkingMode::Hide, "bir iki üç dört beş", "", true);
        assert_eq!(result, vec!["Düşünüyorum... Şu ana kadar 5 kelime düşündüm."]);
        // silent: thinks in the background but no placeholder (same view as off)
        let result = stream_view(ThinkingMode::Silent, "hmm düşünüyorum", "", true);
        assert!(result.is_empty());
        // off: no placeholder
        let result = stream_view(ThinkingMode::Off, "", "", true);
        assert!(result.is_empty());
    }

    /// Verifies `stream_view` once the reply has started: Show includes the spoiler+code block, Hide/Silent show the reply only.
    #[test]
    fn view_reply_started() {
        // show: thinking gets both a spoiler and a code block, plus the reply
        let result = stream_view(ThinkingMode::Show, "düşündüm", "cevap bu", true);
        assert_eq!(result.len(), 3);
        assert!(result[0].starts_with("||") && result[0].ends_with("||"));
        assert!(result[1].starts_with("```"));
        assert_eq!(result[2], "cevap bu");
        // hide: reply only (send_stream adds the button)
        let result = stream_view(ThinkingMode::Hide, "düşündüm", "cevap bu", true);
        assert_eq!(result, vec!["cevap bu"]);
        // silent: reply only, no trace at all (no button either)
        let result = stream_view(ThinkingMode::Silent, "düşündüm", "cevap bu", true);
        assert_eq!(result, vec!["cevap bu"]);
    }


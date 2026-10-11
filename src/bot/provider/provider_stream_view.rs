// the visible part of the reply while streaming is in progress: completed lines (ones
// followed by \n) plus the trailing half-line, but only once it clears HALF_LINE_THRESHOLD
// characters. That way "tep" doesn't sit half-formed as a message that gets deleted on the
// next edit, and a short line doesn't trigger a wasted edit.
fn stream_slice(reply: &str, done: bool) -> &str {
    if done {
        return reply;
    }
    let (complete, partial) = match reply.rfind('\n') {
        Some(i) => (&reply[..i], &reply[i + 1..]),
        None => ("", reply),
    };
    if partial.trim().chars().count() >= HALF_LINE_THRESHOLD {
        reply
    } else {
        complete
    }
}

// the thinking phase: while the reply hasn't started yet and the model is still thinking,
// a placeholder is shown; hide mode gets a live word counter, show mode gets plain
// "Düşünüyorum...". Silent and off modes show nothing (silent runs reasoning in the
// background but leaves no trace; off has none to begin with). Once the reply starts, the
// same message is edited to stream it in. done=false means the stream is still running:
// only completed lines are shown.
/// Renders the current stream state into the list of Discord message bodies it should be.
fn stream_view(mode: ThinkingMode, thought: &str, reply: &str, done: bool) -> Vec<String> {
    let slice = stream_slice(reply, done);
    if slice.trim().is_empty() && !thought.trim().is_empty() {
        return match mode {
            ThinkingMode::Hide => vec![thought_counter(thought)],
            ThinkingMode::Show => vec!["Düşünüyorum...".to_string()],
            ThinkingMode::Silent | ThinkingMode::Off => Vec::new(),
        };
    }
    stream_layout(mode, thought, &parse_reply(slice).lines)
}

// thought blocks + line messages. Lines are passed in from the caller: in the final
// layout, send_stream hands in lines that have already gone through de-duplication
fn stream_layout(mode: ThinkingMode, thought: &str, lines: &[String]) -> Vec<String> {
    let thought = single_line(thought);
    let mut v: Vec<String> = Vec::new();
    if mode == ThinkingMode::Show && !thought.is_empty() {
        // show: both a spoiler and a code block
        for p in split(&thought, MESSAGE_LIMIT - 4) {
            v.push(spoiler(&p));
        }
        v.extend(code_blocks(&thought));
    }
    // in hide/silent/off modes the thought never enters the layout; hide gets a "Show
    // Thought Process" button appended after the reply (send_stream adds it), silent gets
    // no button at all
    v.extend(lines.iter().cloned());
    v
}

// the live counter shown while thinking in hide mode: how many words in so far
fn thought_counter(thought: &str) -> String {
    let n = thought.split_whitespace().count();
    format!("Düşünüyorum... Şu ana kadar {n} kelime düşündüm.")
}

// code-block form of the thinking; more than one block if it exceeds 1900 chars
fn code_blocks(text: &str) -> Vec<String> {
    split(text, MESSAGE_LIMIT - 10)
        .into_iter()
        .map(|p| format!("```\n{p}\n```"))
        .collect()
}

// the ephemeral thought opened by the button: a code block sized to fit in a single message
fn thought_display(text: &str) -> String {
    let note = "\n_(düşünce uzun, kısaltıldı)_";
    let limit = MESSAGE_LIMIT - 12 - note.chars().count();
    let total = text.chars().count();
    let body: String = text.chars().take(limit).collect();
    let mut s = format!("```\n{body}\n```");
    if total > limit {
        s.push_str(note);
    }
    s
}

// thinking shouldn't get a newline per thought; collapsed into a single flowing line
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// reconciles the layout with the messages already open: changed ones are edited, missing
// ones are opened, and if the text got shorter (e.g. the name prefix got stripped) extra
// messages are deleted
/// Only the first message gets a reply target. Network errors are logged, not propagated.
async fn write_stream(
    ctx: &Context,
    channel: &RoomKey,
    sent: &mut Vec<Message>,
    layout: &[String],
    reply_to: Option<&MessageKey>,
) {
    let native = (|| -> Result<_, platform::types::KeyError> {
        Ok((platform::discord::native_room(channel)?,
            reply_to.map(platform::discord::native_message).transpose()?))
    })();
    let (native_channel, native_reply) = match native {
        Ok(native) => native,
        Err(e) => { log::error!("stream send failed ({channel}): {e}"); return; }
    };
    // not repeated in the typing-edit loop: firing it every tick would hit Discord's rate
    // limit; the "typing" indicator is sent once by `reply`, before the model call
    for (i, content) in layout.iter().enumerate() {
        match sent.get_mut(i) {
            Some(m) if m.content != *content => {
                if let Err(e) = m
                    .edit(&ctx.http, EditMessage::new().content(content.clone()))
                    .await
                {
                    log::warn!("edit failed ({channel}): {e}");
                }
            }
            Some(_) => {}
            None => {
                let msg = stream_message(native_channel, content, i, native_reply);
                match native_channel.send_message(&ctx.http, msg).await {
                    Ok(m) => sent.push(m),
                    Err(e) => {
                        log::error!("send failed ({channel}): {e}");
                        break;
                    }
                }
            }
        }
    }
    while sent.len() > layout.len() {
        if let Some(m) = sent.pop() {
            let _ = m.delete(&ctx.http).await;
        }
    }
}

fn stream_message(
    channel: ChannelId,
    content: &str,
    index: usize,
    reply_to: Option<MessageId>,
) -> CreateMessage {
    discord_message(channel, content, None, reply_to.filter(|_| index == 0))
}

/// Best-effort cleanup of provisional messages; individual deletion failures are ignored.
async fn delete_messages(ctx: &Context, messages: Vec<Message>) {
    for m in messages {
        let _ = m.delete(&ctx.http).await;
    }
}

// cache_control is a field Anthropic invented. It can be added safely to every request
// sent to OpenRouter: OpenRouter accepts it as part of its own unified schema and decides
// on its side which models it actually caches for (claude, gemini, ...), silently ignoring
// it on models that don't support it — no need to guess the model name here. Mistral's
// native API, or a custom router given via API_URL, offer no such guarantee: an unknown
// field could get the whole request rejected, so it's only added when the target is the
// openrouter URL.

#[cfg(test)]
mod discord_stream_view_fixtures {
    use super::*;

    #[test]
    fn only_first_stream_message_references_and_pings_reply_author() {
        let channel = ChannelId::new(7);
        let target = MessageId::new(81);
        let layout = stream_layout(ThinkingMode::Show, "hmm", &["selam <@42>".into()]);
        assert_eq!(layout, vec!["||hmm||", "```\nhmm\n```", "selam <@42>"]);
        for (index, content) in layout.iter().enumerate() {
            let actual =
                serde_json::to_value(stream_message(channel, content, index, Some(target))).unwrap();
            assert_eq!(actual["content"].as_str().unwrap().as_bytes(), content.as_bytes());
            if index == 0 {
                // The reference goes on the thought spoiler, not the first reply line.
                assert_eq!(
                    actual["allowed_mentions"],
                    serde_json::json!({
                        "parse": [], "users": [], "roles": [], "replied_user": true
                    })
                );
                assert_eq!(
                    actual["message_reference"],
                    serde_json::json!({
                        "type": 0, "message_id": "81", "channel_id": "7",
                        "guild_id": null, "fail_if_not_exists": null
                    })
                );
            } else {
                assert_eq!(
                    actual["allowed_mentions"],
                    serde_json::json!({"parse": [], "users": [], "roles": []})
                );
                assert!(actual.get("message_reference").is_none());
            }
        }
        let untargeted =
            serde_json::to_value(stream_message(channel, "@everyone", 0, None)).unwrap();
        assert_eq!(
            untargeted["allowed_mentions"],
            serde_json::json!({"parse": [], "users": [], "roles": []})
        );
        assert!(untargeted.get("message_reference").is_none());
    }

    #[test]
    fn thought_modes_render_exact_whitespace_and_markdown_bytes() {
        let thought = " \tbir |iki|\n``` üç  ";
        let lines = vec!["selam 💀".to_string(), "<@42> @everyone".to_string()];
        assert_eq!(
            stream_layout(ThinkingMode::Show, thought, &lines),
            vec![
                "||bir \\|iki\\| ``` üç||",
                "```\nbir |iki| ``` üç\n```",
                "selam 💀",
                "<@42> @everyone",
            ]
        );
        for mode in [ThinkingMode::Hide, ThinkingMode::Silent, ThinkingMode::Off] {
            assert_eq!(stream_layout(mode, thought, &lines), lines);
        }
        for done in [false, true] {
            assert_eq!(
                stream_view(ThinkingMode::Show, thought, "", done),
                vec!["Düşünüyorum..."]
            );
            assert_eq!(
                stream_view(ThinkingMode::Hide, thought, "", done),
                vec!["Düşünüyorum... Şu ana kadar 4 kelime düşündüm."]
            );
            for mode in [ThinkingMode::Silent, ThinkingMode::Off] {
                assert!(stream_view(mode, thought, "", done).is_empty());
            }
        }
    }

    #[test]
    fn thought_split_budgets_and_spoiler_escape_expansion_are_unchanged() {
        let thought = "ğ".repeat(MESSAGE_LIMIT - 3);
        assert_eq!(
            stream_layout(ThinkingMode::Show, &thought, &["he".into()]),
            vec![
                format!("||{}||", "ğ".repeat(MESSAGE_LIMIT - 4)),
                "||ğ||".to_string(),
                format!("```\n{}\n```", "ğ".repeat(MESSAGE_LIMIT - 10)),
                format!("```\n{}\n```", "ğ".repeat(7)),
                "he".to_string(),
            ]
        );
        // Escaping happens after splitting: pipes can expand beyond MESSAGE_LIMIT.
        let pipes = "|".repeat(MESSAGE_LIMIT - 4);
        let layout = stream_layout(ThinkingMode::Show, &pipes, &[]);
        assert_eq!(layout[0], format!("||{}||", "\\|".repeat(MESSAGE_LIMIT - 4)));
        assert_eq!(layout[0].chars().count(), 2 * (MESSAGE_LIMIT - 4) + 4);
    }

    #[test]
    fn half_line_threshold_counts_trimmed_unicode_characters() {
        let below = format!("bir\n  {}  ", "ğ".repeat(HALF_LINE_THRESHOLD - 1));
        let threshold = format!("bir\n  {}  ", "ğ".repeat(HALF_LINE_THRESHOLD));
        assert_eq!(stream_slice(&below, false), "bir");
        assert_eq!(stream_slice(&threshold, false), threshold);
        assert_eq!(stream_slice(&below, true), below);
        assert_eq!(
            stream_view(ThinkingMode::Show, "hmm", "kısa", false),
            vec!["Düşünüyorum..."]
        );
        assert_eq!(
            stream_view(ThinkingMode::Show, "hmm", "kısa", true),
            vec!["||hmm||", "```\nhmm\n```", "kısa"]
        );
    }

    #[test]
    fn thought_button_display_keeps_exact_truncation_and_note() {
        assert_eq!(thought_display("bir\niki"), "```\nbir\niki\n```");
        let note = "\n_(düşünce uzun, kısaltıldı)_";
        let limit = MESSAGE_LIMIT - 12 - note.chars().count();
        let exact = "ğ".repeat(limit);
        assert_eq!(thought_display(&exact), format!("```\n{exact}\n```"));
        assert_eq!(
            thought_display(&format!("{exact}💀")),
            format!("```\n{exact}\n```{note}")
        );
    }
}

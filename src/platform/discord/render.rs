//! Discord presentation, native builders, and byte-for-byte sender fixtures.
use super::{native_message, native_room, native_user};
use crate::platform::capabilities::OutputLimits;
use crate::platform::outgoing::{MentionPolicy, OutgoingMessage, PresentationPayload};
use crate::platform::{PlatformError, PlatformResult};
use crate::{
    single_line, split, spoiler, stream_slice, ThinkingMode, MESSAGE_LIMIT, THOUGHT_BUTTON,
};
#[cfg(test)]
use crate::{stream_early_result, stream_presentation, StreamResult, HALF_LINE_THRESHOLD};
use serenity::all::*;

// Keep the original fixture call shape while making the protocol budget explicit.
fn parse_reply(text: &str) -> crate::Reply {
    crate::parse_reply(text, OutputLimits::DISCORD)
}

// Keep the native mention/reference shape shared by plain sends and stream openings.
#[cfg(test)]
pub(crate) fn discord_message(
    channel: ChannelId,
    text: &str,
    ping: Option<UserId>,
    reply_to: Option<MessageId>,
) -> CreateMessage {
    let mut mentions = CreateAllowedMentions::new();
    if let Some(user) = ping {
        mentions = mentions.users([user]);
    }
    if reply_to.is_some() {
        mentions = mentions.replied_user(true);
    }
    let mut message = CreateMessage::new()
        .content(text)
        .allowed_mentions(mentions);
    if let Some(id) = reply_to {
        message = message.reference_message((channel, id));
    }
    message
}

#[cfg(test)]
mod discord_send_fixtures {
    use super::*;

    #[test]
    fn plain_welcome_and_reply_mention_shapes() {
        let text = "selam <@42> <@&99> @everyone @here 💀";
        let channel = ChannelId::new(7);
        for (ping, reply_to, mentions, reference) in [
            (
                None,
                None,
                serde_json::json!({"parse": [], "users": [], "roles": []}),
                None,
            ),
            (
                Some(UserId::new(42)),
                None,
                serde_json::json!({"parse": [], "users": ["42"], "roles": []}),
                None,
            ),
            (
                None,
                Some(MessageId::new(81)),
                serde_json::json!({
                    "parse": [], "users": [], "roles": [], "replied_user": true
                }),
                Some(serde_json::json!({
                    "type": 0, "message_id": "81", "channel_id": "7",
                    "guild_id": null, "fail_if_not_exists": null
                })),
            ),
            (
                Some(UserId::new(42)),
                Some(MessageId::new(81)),
                serde_json::json!({
                    "parse": [], "users": ["42"], "roles": [], "replied_user": true
                }),
                Some(serde_json::json!({
                    "type": 0, "message_id": "81", "channel_id": "7",
                    "guild_id": null, "fail_if_not_exists": null
                })),
            ),
        ] {
            let actual =
                serde_json::to_value(discord_message(channel, text, ping, reply_to)).unwrap();
            assert_eq!(
                actual["content"].as_str().unwrap().as_bytes(),
                text.as_bytes()
            );
            assert_eq!(actual["allowed_mentions"], mentions);
            assert_eq!(actual.get("message_reference"), reference.as_ref());
            assert!(actual.get("components").is_none());
            assert_eq!(actual["embeds"], serde_json::json!([]));
        }
    }
}

// the thinking phase: while the reply hasn't started yet and the model is still thinking,
// a placeholder is shown; hide mode gets a live word counter, show mode gets plain
// "Düşünüyorum...". Silent and off modes show nothing (silent runs reasoning in the
// background but leaves no trace; off has none to begin with). Once the reply starts, the
// same message is edited to stream it in. done=false means the stream is still running:
// only completed lines are shown.
/// Renders the current stream state into the list of Discord message bodies it should be.
pub(crate) fn stream_view(
    mode: ThinkingMode,
    thought: &str,
    reply: &str,
    done: bool,
) -> Vec<String> {
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
pub(crate) fn stream_layout(mode: ThinkingMode, thought: &str, lines: &[String]) -> Vec<String> {
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
pub(crate) fn thought_counter(thought: &str) -> String {
    let n = thought.split_whitespace().count();
    format!("Düşünüyorum... Şu ana kadar {n} kelime düşündüm.")
}

// code-block form of the thinking; more than one block if it exceeds 1900 chars
pub(crate) fn code_blocks(text: &str) -> Vec<String> {
    split(text, MESSAGE_LIMIT - 10)
        .into_iter()
        .map(|p| format!("```\n{p}\n```"))
        .collect()
}

// the ephemeral thought opened by the button: a code block sized to fit in a single message
pub(crate) fn thought_display(text: &str) -> String {
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

#[cfg(test)]
pub(crate) fn stream_message(
    channel: ChannelId,
    content: &str,
    index: usize,
    reply_to: Option<MessageId>,
) -> CreateMessage {
    discord_message(channel, content, None, reply_to.filter(|_| index == 0))
}

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
                serde_json::to_value(stream_message(channel, content, index, Some(target)))
                    .unwrap();
            assert_eq!(
                actual["content"].as_str().unwrap().as_bytes(),
                content.as_bytes()
            );
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
        assert_eq!(
            layout[0],
            format!("||{}||", "\\|".repeat(MESSAGE_LIMIT - 4))
        );
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
pub(crate) fn thought_button_edit() -> EditMessage {
    EditMessage::new().components(vec![thought_button()])
}

#[cfg(test)]
mod discord_stream_sender_fixtures {
    use super::*;

    #[test]
    fn stealth_suppresses_thought_modes_and_every_live_write() {
        for mode in [
            ThinkingMode::Show,
            ThinkingMode::Hide,
            ThinkingMode::Silent,
            ThinkingMode::Off,
        ] {
            assert_eq!(stream_presentation(mode, false), (mode, true));
            let (stealth_mode, live_edits) = stream_presentation(mode, true);
            let expected = if mode == ThinkingMode::Off {
                ThinkingMode::Off
            } else {
                ThinkingMode::Silent
            };
            assert_eq!(stealth_mode, expected);
            assert!(!live_edits);
            assert!(stream_view(stealth_mode, "gizli düşünce", "", false).is_empty());
            assert_eq!(
                stream_layout(stealth_mode, "gizli düşünce", &["selam".into()]),
                vec!["selam"]
            );
        }
    }

    #[test]
    fn provisional_thinking_and_silence_are_distinct_from_final_layout() {
        for mode in [ThinkingMode::Show, ThinkingMode::Hide] {
            assert!(!stream_view(mode, "bir iki", "", false).is_empty());
            let silent = parse_reply("-");
            assert!(matches!(
                stream_early_result(&silent),
                Some(StreamResult::Silent)
            ));
            // send_stream exits and deletes the placeholder before this Show layout
            // could expose the thought. stream_layout itself does not decide silence.
            if mode == ThinkingMode::Show {
                assert_eq!(
                    stream_layout(mode, "bir iki", &silent.lines),
                    vec!["||bir iki||", "```\nbir iki\n```"]
                );
                assert_eq!(
                    stream_view(mode, "bir iki", "-", true),
                    vec!["||bir iki||", "```\nbir iki\n```"]
                );
            } else {
                assert!(stream_layout(mode, "bir iki", &silent.lines).is_empty());
            }
        }
        assert!(matches!(
            stream_early_result(&parse_reply(" \n ")),
            Some(StreamResult::Empty)
        ));
        assert!(matches!(
            stream_early_result(&parse_reply("tepki: geçersiz")),
            Some(StreamResult::Empty)
        ));
        for text in ["tepki: 💀", "tepki: 💀\n-", "selam\n-", "selam\ntepki: 💀"] {
            assert!(stream_early_result(&parse_reply(text)).is_none());
        }
        let reaction = parse_reply("tepki: 💀\n-");
        assert_eq!(reaction.protocol_text(), "tepki: 💀");
        assert!(stream_layout(ThinkingMode::Hide, "bir iki", &reaction.lines).is_empty());
        assert_eq!(
            stream_layout(ThinkingMode::Show, "bir iki", &reaction.lines),
            vec!["||bir iki||", "```\nbir iki\n```"]
        );
    }

    #[test]
    fn hidden_thought_button_is_components_only_with_native_secondary_style() {
        let actual = serde_json::to_value(thought_button_edit()).unwrap();
        assert_eq!(actual["components"][0]["type"], serde_json::json!(1));
        let button = &actual["components"][0]["components"][0];
        assert_eq!(button["type"], 2);
        assert_eq!(button["style"], 2);
        assert_eq!(button["custom_id"], "show_thought");
        assert_eq!(button["label"], "Düşünce Sürecini Göster");
        assert_eq!(button["disabled"], false);
        assert!(actual.get("content").is_none());
        assert!(actual.get("allowed_mentions").is_none());
        assert!(actual.get("message_reference").is_none());
    }
}

/// Native mention syntax is generated only at the adapter boundary.
pub(crate) fn outgoing_text<'a>(
    text: &'a str,
    policy: &MentionPolicy<'_>,
) -> PlatformResult<std::borrow::Cow<'a, str>> {
    match policy.prefix_user {
        Some(user) => Ok(std::borrow::Cow::Owned(format!(
            "<@{}> {text}",
            native_user(user)?
        ))),
        None => Ok(std::borrow::Cow::Borrowed(text)),
    }
}

pub(crate) fn allowed_mentions(
    policy: &MentionPolicy<'_>,
) -> PlatformResult<CreateAllowedMentions> {
    let users = policy
        .users
        .iter()
        .map(native_user)
        .collect::<Result<Vec<_>, _>>()?;
    let mut mentions = CreateAllowedMentions::new().users(users);
    if policy.reply_user {
        mentions = mentions.replied_user(true);
    }
    Ok(mentions)
}

fn thought_button() -> CreateActionRow {
    CreateActionRow::Buttons(vec![CreateButton::new(THOUGHT_BUTTON)
        .label("Düşünce Sürecini Göster")
        .style(ButtonStyle::Secondary)])
}

/// Pure builder helper shared by transport and serialized fixtures.
pub(crate) fn outgoing_message(
    channel: ChannelId,
    body: &OutgoingMessage<'_>,
) -> PlatformResult<CreateMessage> {
    let content = outgoing_text(&body.text, &body.mentions)?.into_owned();
    let mut message = CreateMessage::new()
        .content(content)
        .allowed_mentions(allowed_mentions(&body.mentions)?);
    if let Some(target) = body.reply_to {
        if native_room(&target.room)? != channel {
            return Err(PlatformError::Unsupported("cross-room replies"));
        }
        message = message.reference_message((channel, native_message(&target.id)?));
    }
    match body.presentation {
        Some(PresentationPayload::Card { title, description }) => {
            message = message.embed(CreateEmbed::new().title(title).description(description))
        }
        Some(PresentationPayload::ThoughtReveal) => {
            message = message.components(vec![thought_button()])
        }
        None => {}
    }
    Ok(message)
}

pub(crate) fn outgoing_edit(body: &OutgoingMessage<'_>) -> PlatformResult<EditMessage> {
    if matches!(body.presentation, Some(PresentationPayload::ThoughtReveal)) {
        return Ok(thought_button_edit());
    }
    let mut edit = EditMessage::new()
        .content(outgoing_text(&body.text, &body.mentions)?.into_owned())
        .allowed_mentions(allowed_mentions(&body.mentions)?);
    if let Some(PresentationPayload::Card { title, description }) = body.presentation {
        edit = edit.embed(CreateEmbed::new().title(title).description(description));
    }
    Ok(edit)
}

#[cfg(test)]
mod outgoing_policy_fixtures {
    use super::*;
    use crate::platform::types::{MessageKey, MessageRef, RoomKey, UserKey};

    #[test]
    fn generic_policy_preserves_native_reply_and_welcome_shapes() {
        let channel = ChannelId::new(7);
        let user = UserKey::discord(42);
        let target = MessageRef {
            id: MessageKey::discord(81),
            room: RoomKey::discord(7),
            author: None,
        };
        let mut body = OutgoingMessage::text("selam @everyone <@&99>");
        assert_eq!(
            serde_json::to_value(outgoing_message(channel, &body).unwrap()).unwrap(),
            serde_json::to_value(discord_message(channel, &body.text, None, None)).unwrap()
        );
        body.reply_to = Some(&target);
        body.mentions.reply_user = true;
        body.mentions.users = std::slice::from_ref(&user);
        body.mentions.prefix_user = Some(&user);
        assert_eq!(
            serde_json::to_value(outgoing_message(channel, &body).unwrap()).unwrap(),
            serde_json::to_value(discord_message(
                channel,
                "<@42> selam @everyone <@&99>",
                Some(UserId::new(42)),
                Some(MessageId::new(81))
            ))
            .unwrap()
        );
        body.presentation = Some(PresentationPayload::ThoughtReveal);
        assert_eq!(outgoing_edit(&body).unwrap(), thought_button_edit());
    }
}

//! Slack ingress normalization. Mutable messages and automated traffic never learn.
use crate::platform::{
    interface::{PlatformError, PlatformResult},
    types::*,
};
use serde_json::Value;

pub(crate) fn room(team: &str, channel: &str) -> PlatformResult<RoomRef> {
    let kind = match channel.as_bytes().first() {
        Some(b'C') => RoomKind::Channel,
        Some(b'G') => RoomKind::Group,
        _ => return Err(PlatformError::Unsupported("Slack direct or unknown room")),
    };
    Ok(RoomRef {
        id: RoomKey::new(PlatformKind::Slack, &[team, channel])?,
        space: Some(SpaceKey::new(PlatformKind::Slack, &[team])?),
        kind,
        name: None,
        thread: None,
    })
}

pub(crate) fn user(team: &str, id: &str) -> PlatformResult<UserRef> {
    Ok(UserRef {
        id: UserKey::new(PlatformKind::Slack, &[team, id])?,
        display_name: id.to_owned(),
        username: None,
        automation: Automation::Human,
        stability: IdentityStability::Persistent,
    })
}

/// Slack's microsecond timestamp is an opaque identity, not a floating-point ID.
/// Only the separate wall-clock field is truncated to milliseconds.
pub(crate) fn timestamp_ms(ts: &str) -> i64 {
    parse_timestamp_ms(ts).unwrap_or(0)
}

fn parse_timestamp_ms(ts: &str) -> Option<i64> {
    let (seconds, fraction) = ts.split_once('.').unwrap_or((ts, ""));
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (ts.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let seconds = seconds.parse::<i64>().ok()?;
    let mut milliseconds = 0_i64;
    let mut digits = fraction.bytes();
    for _ in 0..3 {
        milliseconds = milliseconds * 10 + i64::from(digits.next().unwrap_or(b'0') - b'0');
    }
    seconds.checked_mul(1_000)?.checked_add(milliseconds)
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str().filter(|value| !value.is_empty())
}

fn channel_event(event: &Value) -> Option<&str> {
    if matches!(string(event, "channel_type"), Some("im" | "mpim")) {
        return None;
    }
    string(event, "channel")
        .filter(|channel| matches!(channel.as_bytes().first(), Some(b'C' | b'G')))
}

fn automated(event: &Value) -> bool {
    string(event, "bot_id").is_some()
        || event
            .get("bot_profile")
            .is_some_and(|value| !value.is_null())
        || event.get("is_bot").and_then(Value::as_bool) == Some(true)
        || event.get("is_app_user").and_then(Value::as_bool) == Some(true)
        || event
            .get("user")
            .and_then(|user| user.get("is_bot"))
            .and_then(Value::as_bool)
            == Some(true)
}

fn message_key(team: &str, channel: &str, ts: &str) -> PlatformResult<MessageKey> {
    Ok(MessageKey::new(PlatformKind::Slack, &[team, channel, ts])?)
}

fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        result.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(after) = rest.strip_prefix("&amp;") {
            result.push('&');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&lt;") {
            result.push('<');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&gt;") {
            result.push('>');
            rest = after;
        } else {
            result.push('&');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}

pub(super) fn normalize_text(team: &str, text: &str) -> PlatformResult<(String, Vec<UserKey>)> {
    let mut result = String::with_capacity(text.len());
    let mut mentions = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        result.push_str(&decode_entities(&rest[..start]));
        rest = &rest[start..];
        let Some(end) = rest.find('>') else {
            break;
        };
        let token = &rest[1..end];
        let (target, label) = token
            .split_once('|')
            .map_or((token, None), |(target, label)| (target, Some(label)));
        if let Some(id) = target.strip_prefix('@').filter(|id| !id.is_empty()) {
            let key = UserKey::new(PlatformKind::Slack, &[team, id])?;
            if !mentions.contains(&key) {
                mentions.push(key);
            }
            result.push('@');
            result.push_str(&decode_entities(
                label.unwrap_or(id).trim_start_matches('@'),
            ));
        } else if let Some(id) = target.strip_prefix('#') {
            result.push('#');
            result.push_str(&decode_entities(
                label.unwrap_or(id).trim_start_matches('#'),
            ));
        } else if matches!(target, "!here" | "!channel" | "!everyone") {
            result.push('@');
            result.push_str(&target[1..]);
        } else if target.starts_with("!subteam^") {
            result.push_str(&decode_entities(label.unwrap_or("@group")));
        } else if target.starts_with("!date^")
            || target.starts_with("https://")
            || target.starts_with("http://")
            || target.starts_with("mailto:")
        {
            result.push_str(&decode_entities(label.unwrap_or(target)));
        } else {
            result.push_str(&decode_entities(&rest[..=end]));
        }
        rest = &rest[end + 1..];
    }
    result.push_str(&decode_entities(rest));
    Ok((result, mentions))
}

fn attachments(connection: &ConnectionId, event: &Value) -> Vec<AttachmentRef> {
    event
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let id = string(file, "id")?;
            let mime = string(file, "mimetype");
            let kind = match mime {
                Some(mime) if mime.starts_with("image/") => MediaKind::Image,
                Some(mime) if mime.starts_with("audio/") => MediaKind::Audio,
                Some(mime) if mime.starts_with("video/") => MediaKind::Video,
                _ => MediaKind::File,
            };
            Some(AttachmentRef {
                kind,
                mime: mime.map(str::to_owned),
                size: file.get("size").and_then(Value::as_u64),
                source: MediaSource::Native {
                    connection: connection.clone(),
                    id: id.to_owned(),
                },
            })
        })
        .collect()
}

pub(crate) fn decode(
    connection: &ConnectionId,
    team: &str,
    self_user: &UserKey,
    event: &Value,
) -> PlatformResult<Option<ChatEvent>> {
    match string(event, "type") {
        Some("message" | "app_mention") => {
            // Deliberately do not unwrap message_changed's nested message: edits
            // and deletes must not become new training observations.
            if automated(event)
                || event.get("subtype").is_some_and(|subtype| {
                    !subtype.is_null() && subtype.as_str() != Some("file_share")
                })
            {
                return Ok(None);
            }
            let (Some(channel), Some(author), Some(ts)) = (
                channel_event(event),
                string(event, "user"),
                string(event, "ts"),
            ) else {
                return Ok(None);
            };
            if parse_timestamp_ms(ts).is_none() {
                return Ok(None);
            }
            let author = user(team, author)?;
            if &author.id == self_user {
                return Ok(None);
            }
            let mut room = room(team, channel)?;
            let reply_to = if let Some(thread_ts) = string(event, "thread_ts") {
                if parse_timestamp_ms(thread_ts).is_none() {
                    return Ok(None);
                }
                room.thread = Some(ThreadKey::new(
                    PlatformKind::Slack,
                    &[team, channel, thread_ts],
                )?);
                if thread_ts != ts {
                    Some(MessageRef {
                        id: message_key(team, channel, thread_ts)?,
                        room: room.id.clone(),
                        author: None,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let (text, mentions) = normalize_text(
                team,
                event.get("text").and_then(Value::as_str).unwrap_or(""),
            )?;
            Ok(Some(ChatEvent::MessageCreated(IncomingMessage {
                id: message_key(team, channel, ts)?,
                room,
                author,
                sent_at_unix_ms: timestamp_ms(ts),
                text,
                mentions,
                reply_to,
                attachments: attachments(connection, event),
            })))
        }
        Some("member_joined_channel") => {
            if automated(event) {
                return Ok(None);
            }
            let (Some(channel), Some(id)) = (channel_event(event), string(event, "user")) else {
                return Ok(None);
            };
            let user = user(team, id)?;
            if &user.id == self_user {
                return Ok(None);
            }
            Ok(Some(ChatEvent::MemberJoined {
                room: room(team, channel)?,
                user,
            }))
        }
        Some("reaction_added") => {
            if automated(event) {
                return Ok(None);
            }
            let (Some(actor), Some(reaction), Some(item)) = (
                string(event, "user"),
                string(event, "reaction"),
                event.get("item"),
            ) else {
                return Ok(None);
            };
            if string(item, "type") != Some("message") {
                return Ok(None);
            }
            let (Some(channel), Some(ts)) = (channel_event(item), string(item, "ts")) else {
                return Ok(None);
            };
            if parse_timestamp_ms(ts).is_none() {
                return Ok(None);
            }
            let actor = user(team, actor)?;
            if &actor.id == self_user {
                return Ok(None);
            }
            let room = room(team, channel)?;
            let target = MessageRef {
                id: message_key(team, channel, ts)?,
                room: room.id.clone(),
                author: string(event, "item_user")
                    .map(|id| user(team, id))
                    .transpose()?
                    .map(|user| user.id),
            };
            Ok(Some(ChatEvent::ReactionAdded(ReactionEvent {
                room,
                actor,
                target,
                emoji: Emoji::Custom {
                    platform: PlatformKind::Slack,
                    id: reaction.to_owned(),
                    label: Some(reaction.to_owned()),
                    animated: false,
                },
                original_text: None,
            })))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connection() -> ConnectionId {
        ConnectionId::new("slack-test")
    }

    fn decode_fixture(event: Value) -> Option<ChatEvent> {
        decode(
            &connection(),
            "T123",
            &UserKey::new(PlatformKind::Slack, &["T123", "UBOT"]).unwrap(),
            &event,
        )
        .unwrap()
    }

    fn message(event: Value) -> IncomingMessage {
        match decode_fixture(event).unwrap() {
            ChatEvent::MessageCreated(message) => message,
            other => panic!("expected message, received {other:?}"),
        }
    }

    #[test]
    fn ordinary_message_decodes_entities_and_structured_mentions() {
        let message = message(json!({
            "type": "message", "channel": "C456", "user": "U789",
            "text": "Hi <@U111> &amp; <@U111> &lt;literal&gt; <#C999|general> <https://example.com|example>",
            "ts": "1712345678.123456"
        }));
        assert_eq!(message.text, "Hi @U111 & @U111 <literal> #general example");
        assert_eq!(message.author.display_name, "U789");
        assert_eq!(message.author.automation, Automation::Human);
        assert_eq!(message.sent_at_unix_ms, 1_712_345_678_123);
        assert_eq!(
            message.mentions,
            vec![UserKey::new(PlatformKind::Slack, &["T123", "U111"]).unwrap()]
        );
        assert_eq!(message.room.kind, RoomKind::Channel);
        assert!(message.room.thread.is_none());
        assert!(message.reply_to.is_none());
        assert!(message.attachments.is_empty());
    }

    #[test]
    fn thread_reply_keeps_exact_root_and_reply_timestamps() {
        let message = message(json!({
            "type": "message", "channel": "G456", "user": "U789", "text": "reply",
            "ts": "1712345678.000009", "thread_ts": "1712345677.000001"
        }));
        assert_eq!(message.room.kind, RoomKind::Group);
        assert_eq!(
            message.room.thread.as_ref().unwrap().as_str(),
            "slack:thread:T123:G456:1712345677.000001"
        );
        assert_eq!(
            message.id.as_str(),
            "slack:message:T123:G456:1712345678.000009"
        );
        let reply = message.reply_to.unwrap();
        assert_eq!(
            reply.id.as_str(),
            "slack:message:T123:G456:1712345677.000001"
        );
        assert_eq!(reply.room, message.room.id);
        assert!(reply.author.is_none());
        assert_eq!(message.sent_at_unix_ms, 1_712_345_678_000);
    }

    #[test]
    fn thread_root_does_not_reply_to_itself() {
        let message = message(json!({
            "type": "message", "channel": "C456", "user": "U789", "text": "root",
            "ts": "1712345678.000009", "thread_ts": "1712345678.000009"
        }));
        assert!(message.room.thread.is_some());
        assert!(message.reply_to.is_none());
    }

    #[test]
    fn bot_message_and_self_and_bot_metadata_are_filtered() {
        for event in [
            json!({"type":"message", "subtype":"bot_message", "channel":"C456", "user":"U789", "text":"bot", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"C456", "user":"UBOT", "text":"self", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"C456", "user":"U789", "bot_id":"B123", "text":"bot", "ts":"1712345678.123456"}),
            json!({"type":"app_mention", "channel":"C456", "user":"U789", "bot_profile":{}, "text":"bot", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"C456", "user":"U789", "is_bot":true, "text":"bot", "ts":"1712345678.123456"}),
        ] {
            assert!(decode_fixture(event).is_none());
        }
    }

    #[test]
    fn message_changed_explicitly_never_relearns_nested_human_message() {
        assert!(decode_fixture(json!({
            "type":"message", "subtype":"message_changed", "channel":"C456",
            "ts":"1712345679.123456", "user":"U789",
            "message":{"type":"message", "user":"U789", "text":"edited", "ts":"1712345678.123456"}
        }))
        .is_none());
        for subtype in [
            "message_deleted",
            "thread_broadcast",
            "channel_join",
            "me_message",
            "unknown_future_subtype",
        ] {
            assert!(decode_fixture(json!({
                "type":"message", "subtype":subtype, "channel":"C456", "user":"U789",
                "text":"not a new ordinary message", "ts":"1712345678.123456"
            }))
            .is_none());
        }
    }

    #[test]
    fn file_share_retains_only_native_file_id_and_media_metadata() {
        let message = message(json!({
            "type":"message", "subtype":"file_share", "channel":"C456", "user":"U789",
            "text":"", "ts":"1712345678.123456",
            "files":[
                {"id":"F123", "mimetype":"image/png", "size":12345, "url_private":"https://files.slack.com/private/secret", "url_private_download":"https://files.slack.com/private/download"},
                {"id":"F124", "mimetype":"application/pdf", "size":456},
                {"mimetype":"image/png", "url_private":"https://files.slack.com/no-id"}
            ]
        }));
        assert_eq!(message.attachments.len(), 2);
        let image = &message.attachments[0];
        assert_eq!(image.kind, MediaKind::Image);
        assert_eq!(image.mime.as_deref(), Some("image/png"));
        assert_eq!(image.size, Some(12345));
        match &image.source {
            MediaSource::Native {
                connection: source_connection,
                id,
            } => {
                assert_eq!(source_connection, &connection());
                assert_eq!(id, "F123");
            }
            other => panic!("expected native file reference, received {other:?}"),
        }
        assert_eq!(message.attachments[1].kind, MediaKind::File);
        assert!(!serde_json::to_string(&message.attachments)
            .unwrap()
            .contains("https://"));
    }

    #[test]
    fn member_joined_channel_normalizes_workspace_identity() {
        match decode_fixture(json!({
            "type":"member_joined_channel", "channel":"G456", "channel_type":"G", "user":"U789"
        }))
        .unwrap()
        {
            ChatEvent::MemberJoined { room, user } => {
                assert_eq!(room.id.as_str(), "slack:room:T123:G456");
                assert_eq!(room.space.unwrap().as_str(), "slack:space:T123");
                assert_eq!(user.id.as_str(), "slack:user:T123:U789");
                assert_eq!(user.display_name, "U789");
            }
            other => panic!("expected member join, received {other:?}"),
        }
    }

    #[test]
    fn app_mention_preserves_bot_mention_for_direct_address_detection() {
        let message = message(json!({
            "type":"app_mention", "channel":"C456", "user":"U789",
            "text":"<@UBOT> help &gt; please", "ts":"1712345678.123456"
        }));
        assert_eq!(message.text, "@UBOT help > please");
        assert_eq!(
            message.mentions,
            vec![UserKey::new(PlatformKind::Slack, &["T123", "UBOT"]).unwrap()]
        );
    }

    #[test]
    fn reaction_added_keeps_custom_name_actor_and_target_without_text() {
        match decode_fixture(json!({
            "type":"reaction_added", "user":"U789", "reaction":"party_parrot", "item_user":"U111",
            "item":{"type":"message", "channel":"C456", "ts":"1712345678.000001"}
        }))
        .unwrap()
        {
            ChatEvent::ReactionAdded(reaction) => {
                assert_eq!(reaction.actor.id.as_str(), "slack:user:T123:U789");
                assert_eq!(
                    reaction.target.id.as_str(),
                    "slack:message:T123:C456:1712345678.000001"
                );
                assert_eq!(
                    reaction.target.author.unwrap().as_str(),
                    "slack:user:T123:U111"
                );
                assert!(reaction.original_text.is_none());
                match reaction.emoji {
                    Emoji::Custom {
                        platform,
                        id,
                        label,
                        animated,
                    } => {
                        assert_eq!(platform, PlatformKind::Slack);
                        assert_eq!(id, "party_parrot");
                        assert_eq!(label.as_deref(), Some("party_parrot"));
                        assert!(!animated);
                    }
                    other => panic!("expected Slack emoji name, received {other:?}"),
                }
            }
            other => panic!("expected reaction, received {other:?}"),
        }
    }

    #[test]
    fn keys_round_trip_exact_timestamps_and_scope_every_identity_by_workspace() {
        let event = json!({
            "type":"message", "channel":"C456", "user":"U789", "text":"hello",
            "ts":"1712345678.000001", "thread_ts":"1712345677.000099"
        });
        let first = message(event.clone());
        let second = match decode(
            &connection(),
            "TOTHER",
            &UserKey::new(PlatformKind::Slack, &["TOTHER", "UBOT"]).unwrap(),
            &event,
        )
        .unwrap()
        .unwrap()
        {
            ChatEvent::MessageCreated(message) => message,
            other => panic!("expected message, received {other:?}"),
        };
        assert_ne!(first.id, second.id);
        assert_ne!(first.room.id, second.room.id);
        assert_ne!(first.room.space, second.room.space);
        assert_ne!(first.room.thread, second.room.thread);
        assert_ne!(first.author.id, second.author.id);
        assert_eq!(first.id.as_str().parse::<MessageKey>().unwrap(), first.id);
        assert_eq!(
            first.room.id.as_str().parse::<RoomKey>().unwrap(),
            first.room.id
        );
        assert_eq!(
            first.author.id.as_str().parse::<UserKey>().unwrap(),
            first.author.id
        );
        let thread = first.room.thread.unwrap();
        assert_eq!(thread.as_str().parse::<ThreadKey>().unwrap(), thread);
        assert_eq!(
            first
                .id
                .components()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            ["T123", "C456", "1712345678.000001"]
        );
        assert_eq!(
            thread.components().collect::<Result<Vec<_>, _>>().unwrap(),
            ["T123", "C456", "1712345677.000099"]
        );
        let next = message(
            json!({"type":"message", "channel":"C456", "user":"U789", "text":"hello", "ts":"1712345678.000002"}),
        );
        assert_ne!(first.id, next.id);
        assert_eq!(first.sent_at_unix_ms, next.sent_at_unix_ms);
    }

    #[test]
    fn timestamp_conversion_never_uses_floating_point_or_rounds_up() {
        assert_eq!(timestamp_ms("1712345678.999999"), 1_712_345_678_999);
        assert_eq!(timestamp_ms("1712345678.1"), 1_712_345_678_100);
        assert_eq!(timestamp_ms("1712345678.01"), 1_712_345_678_010);
        assert_eq!(timestamp_ms("1712345678"), 1_712_345_678_000);
        assert_eq!(timestamp_ms("9007199254740.999999"), 9_007_199_254_740_999);
        for invalid in [
            "",
            "nan",
            "1.x",
            "1.",
            "1.2.3",
            "-1.0",
            "9223372036854775807.0",
        ] {
            assert_eq!(timestamp_ms(invalid), 0);
        }
    }

    #[test]
    fn direct_messages_and_unsupported_or_malformed_events_are_excluded() {
        for event in [
            json!({"type":"message", "channel":"D456", "user":"U789", "text":"private", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"G456", "channel_type":"mpim", "user":"U789", "text":"private", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"C456", "text":"no actor", "ts":"1712345678.123456"}),
            json!({"type":"message", "channel":"C456", "user":"U789", "ts":"not-a-timestamp"}),
            json!({"type":"message", "channel":"C456", "user":"U789", "ts":"1712345678.123456", "thread_ts":"invalid"}),
            json!({"type":"reaction_added", "user":"U789", "reaction":"thumbsup", "item":{"type":"file", "file":"F123"}}),
            json!({"type":"unknown_event"}),
        ] {
            assert!(decode_fixture(event).is_none());
        }
        assert!(room("T123", "D456").is_err());
    }

    #[test]
    fn escaped_mentions_are_literal_and_entities_decode_once() {
        let message = message(json!({
            "type":"message", "channel":"C456", "user":"U789", "ts":"1712345678.123456",
            "text":"&lt;@U111&gt; &amp;lt; <!here> <!subteam^S123|@team> <mailto:test@example.com|email>"
        }));
        assert_eq!(message.text, "<@U111> &lt; @here @team email");
        assert!(message.mentions.is_empty());
    }
}

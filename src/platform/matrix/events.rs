//! Plaintext Matrix JSON ingress. Native IDs are encoded only by canonical constructors.
use crate::platform::{
    interface::{PlatformError, PlatformResult},
    types::*,
};
use serde_json::Value;

fn invalid(field: &str) -> PlatformError {
    PlatformError::Transport(format!("invalid Matrix event: {field}"))
}

pub(crate) fn valid_native(native: &str, sigil: char) -> bool {
    native.starts_with(sigil)
        && native.len() > 1
        && !native.chars().any(|c| c.is_whitespace() || c.is_control())
        && (sigil == '$'
            || (sigil == '!' && !native.contains(':'))
            || native[1..]
                .split_once(':')
                .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty()))
}

pub(crate) fn room_ref(native: &str) -> PlatformResult<RoomRef> {
    if !valid_native(native, '!') {
        return Err(invalid("room ID"));
    }
    Ok(RoomRef::from_key(RoomKey::new(
        PlatformKind::Matrix,
        &[native],
    )?))
}

pub(crate) fn user_ref(native: &str, name: Option<&str>) -> PlatformResult<UserRef> {
    if !valid_native(native, '@') {
        return Err(invalid("user ID"));
    }
    Ok(UserRef {
        id: UserKey::new(PlatformKind::Matrix, &[native])?,
        display_name: name.unwrap_or(native).to_owned(),
        username: Some(native.to_owned()),
        automation: Automation::Unknown,
        stability: IdentityStability::Persistent,
    })
}

pub(crate) fn event_key(native: &str) -> PlatformResult<EventKey> {
    if !valid_native(native, '$') {
        return Err(invalid("event ID"));
    }
    Ok(EventKey::new(PlatformKind::Matrix, &[native])?)
}

pub(crate) fn message_key(native: &str) -> PlatformResult<MessageKey> {
    if !valid_native(native, '$') {
        return Err(invalid("message ID"));
    }
    Ok(MessageKey::new(PlatformKind::Matrix, &[native])?)
}

fn required<'a>(value: &'a Value, field: &str) -> PlatformResult<&'a str> {
    value[field].as_str().ok_or_else(|| invalid(field))
}

fn reply_target(content: &Value, room: &RoomRef) -> PlatformResult<Option<MessageRef>> {
    let Some(reply) = content
        .get("m.relates_to")
        .and_then(|v| v.get("m.in_reply_to"))
    else {
        return Ok(None);
    };
    Ok(Some(MessageRef {
        id: message_key(required(reply, "event_id")?)?,
        room: room.id.clone(),
        author: None,
    }))
}

// Strip only a genuine Matrix reply fallback, never an ordinary quoted message.
fn without_reply_fallback(text: &str) -> &str {
    let mut remaining = text;
    let mut quoted = false;
    while remaining.starts_with("> ")
        || remaining.starts_with(">\n")
        || remaining.starts_with(">\r\n")
    {
        let Some(newline) = remaining.find('\n') else {
            return text;
        };
        remaining = &remaining[newline + 1..];
        quoted = true;
    }
    if quoted {
        if let Some(body) = remaining
            .strip_prefix("\r\n")
            .or_else(|| remaining.strip_prefix('\n'))
        {
            return body;
        }
    }
    text
}

fn percent_decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

fn html_unescape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                .or_else(|| {
                    entity
                        .strip_prefix('#')
                        .and_then(|digits| digits.parse().ok())
                })
                .and_then(char::from_u32),
        };
        if let Some(decoded) = decoded {
            output.push(decoded);
        } else {
            output.push_str(&rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

// Read actual href attributes, not matrix.to strings in arbitrary visible text.
fn href(mut attributes: &str) -> Option<&str> {
    while !attributes.is_empty() {
        attributes = attributes.trim_start();
        let end = attributes
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(attributes.len());
        if end == 0 {
            return None;
        }
        let name = &attributes[..end];
        attributes = attributes[end..].trim_start();
        if !attributes.starts_with('=') {
            continue;
        }
        attributes = attributes[1..].trim_start();
        let (value, rest) = if let Some(quote) = attributes
            .chars()
            .next()
            .filter(|c| *c == '\'' || *c == '"')
        {
            let after = &attributes[1..];
            let end = after.find(quote)?;
            (&after[..end], &after[end + 1..])
        } else {
            let end = attributes
                .find(char::is_whitespace)
                .unwrap_or(attributes.len());
            (&attributes[..end], &attributes[end..])
        };
        if name.eq_ignore_ascii_case("href") {
            return Some(value);
        }
        attributes = rest;
    }
    None
}

fn add_mention(mentions: &mut Vec<UserKey>, native: &str) {
    if !valid_native(native, '@') {
        return;
    }
    if let Ok(user) = UserKey::new(PlatformKind::Matrix, &[native]) {
        if !mentions.contains(&user) {
            mentions.push(user);
        }
    }
}

fn mentions(content: &Value, is_reply: bool) -> Vec<UserKey> {
    let mut mentions = Vec::new();
    if let Some(users) = content["m.mentions"]["user_ids"].as_array() {
        for user in users.iter().filter_map(Value::as_str) {
            add_mention(&mut mentions, user);
        }
    }
    if content["format"].as_str() != Some("org.matrix.custom.html") {
        return mentions;
    }
    let Some(mut html) = content["formatted_body"].as_str() else {
        return mentions;
    };
    let mut reply_depth = 0usize;
    while let Some(start) = html.find('<') {
        html = &html[start + 1..];
        let Some(end) = html.find('>') else { break };
        let tag = &html[..end];
        html = &html[end + 1..];
        let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
        let name = &tag[..name_end];
        if is_reply && name.eq_ignore_ascii_case("mx-reply") {
            reply_depth += 1;
        } else if is_reply && name.eq_ignore_ascii_case("/mx-reply") {
            reply_depth = reply_depth.saturating_sub(1);
        } else if reply_depth == 0 && name.eq_ignore_ascii_case("a") {
            if let Some(link) = href(&tag[name_end..]) {
                let link = html_unescape(link);
                if let Some(fragment) = link.strip_prefix("https://matrix.to/#/") {
                    let fragment = fragment.split('?').next().unwrap_or(fragment);
                    if let Some(native) = percent_decode(fragment) {
                        add_mention(&mut mentions, &native);
                    }
                }
            }
        }
    }
    mentions
}

fn valid_mxc(value: &str) -> bool {
    value
        .strip_prefix("mxc://")
        .and_then(|v| v.split_once('/'))
        .is_some_and(|(server, media)| {
            !server.is_empty()
                && !media.is_empty()
                && !media.contains('/')
                && !value.contains(['?', '#'])
                && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        })
}

pub(crate) fn decode_event(
    connection: &ConnectionId,
    room: &RoomRef,
    own: &UserKey,
    raw: &Value,
    image_analysis: bool,
) -> PlatformResult<Option<ChatEvent>> {
    let event_type = raw["type"].as_str().unwrap_or("");
    if event_type == "m.room.encrypted" {
        return Err(PlatformError::Unsupported(
            "encrypted rooms not supported yet",
        ));
    }
    if !matches!(
        event_type,
        "m.room.message" | "m.reaction" | "m.room.member"
    ) {
        return Ok(None);
    }
    let native_id = required(raw, "event_id")?;
    if !valid_native(native_id, '$') {
        return Err(invalid("event ID"));
    }
    let sent_at = raw["origin_server_ts"]
        .as_i64()
        .filter(|v| *v >= 0)
        .ok_or_else(|| invalid("origin_server_ts"))?;
    let author = user_ref(required(raw, "sender")?, None)?;
    if author.id == *own {
        return Ok(None);
    }
    if room.id.platform() != PlatformKind::Matrix {
        return Err(invalid("room platform"));
    }
    if let Some(native_room) = raw.get("room_id") {
        if room_ref(native_room.as_str().ok_or_else(|| invalid("room_id"))?)?.id != room.id {
            return Err(invalid("room_id mismatch"));
        }
    }
    let content = &raw["content"];
    match event_type {
        "m.room.member" => {
            if content["membership"].as_str() != Some("join")
                || raw["unsigned"]["prev_content"]["membership"]
                    .as_str()
                    .or_else(|| raw["prev_content"]["membership"].as_str())
                    == Some("join")
            {
                return Ok(None);
            }
            let user = user_ref(required(raw, "state_key")?, content["displayname"].as_str())?;
            if user.id == *own {
                return Ok(None);
            }
            Ok(Some(ChatEvent::MemberJoined {
                room: room.clone(),
                user,
            }))
        }
        "m.reaction" => {
            let relation = &content["m.relates_to"];
            if relation["rel_type"].as_str() != Some("m.annotation") {
                return Ok(None);
            }
            let emoji = required(relation, "key")?;
            if emoji.is_empty() {
                return Err(invalid("reaction key"));
            }
            Ok(Some(ChatEvent::ReactionAdded(ReactionEvent {
                room: room.clone(),
                actor: author,
                target: MessageRef {
                    id: message_key(required(relation, "event_id")?)?,
                    room: room.id.clone(),
                    author: None,
                },
                emoji: Emoji::Unicode(emoji.to_owned()),
                original_text: None,
            })))
        }
        "m.room.message" => {
            if content["m.relates_to"]["rel_type"].as_str() == Some("m.replace") {
                return Ok(None);
            }
            let msgtype = content["msgtype"].as_str().unwrap_or("");
            if !matches!(msgtype, "m.text" | "m.notice" | "m.emote" | "m.image") {
                return Ok(None);
            }
            if msgtype == "m.image" && !image_analysis {
                return Ok(None);
            }
            let text = required(content, "body")?;
            let reply_to = reply_target(content, room)?;
            let text = if reply_to.is_some() {
                without_reply_fallback(text)
            } else {
                text
            };
            let attachments = if msgtype == "m.image" {
                if content.get("file").is_some() {
                    return Err(PlatformError::Unsupported(
                        "encrypted rooms not supported yet",
                    ));
                }
                let url = required(content, "url")?;
                if !valid_mxc(url) {
                    return Err(invalid("image mxc URL"));
                }
                vec![AttachmentRef {
                    kind: MediaKind::Image,
                    mime: content["info"]["mimetype"].as_str().map(str::to_owned),
                    size: content["info"]["size"].as_u64(),
                    source: MediaSource::Native {
                        connection: connection.clone(),
                        id: url.to_owned(),
                    },
                }]
            } else {
                Vec::new()
            };
            Ok(Some(ChatEvent::MessageCreated(IncomingMessage {
                id: message_key(native_id)?,
                room: room.clone(),
                author,
                sent_at_unix_ms: sent_at,
                text: text.to_owned(),
                mentions: mentions(content, reply_to.is_some()),
                reply_to,
                attachments,
            })))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(kind: &str, content: Value) -> Value {
        json!({"type":kind,"event_id":"$event/+%:server","sender":"@alice:example.org","origin_server_ts":42,"content":content})
    }
    fn decode(raw: &Value, analysis: bool) -> PlatformResult<Option<ChatEvent>> {
        decode_event(
            &ConnectionId::new("matrix-main"),
            &room_ref("!room:example.org")?,
            &user_ref("@bot:example.org", None)?.id,
            raw,
            analysis,
        )
    }
    fn message(raw: &Value) -> IncomingMessage {
        match decode(raw, true).unwrap().unwrap() {
            ChatEvent::MessageCreated(message) => message,
            _ => panic!("not a message"),
        }
    }

    #[test]
    fn modern_opaque_room_ids_have_no_required_server_component() {
        let native = "!46QuI0a_GwzM_rSg9OqHoSP7LffHmVKH0X53xvuV6BE";
        let room = room_ref(native).unwrap();
        assert_eq!(room.id.components().next().unwrap().unwrap(), native);
        assert!(!valid_native("!", '!'));
    }

    #[test]
    fn canonical_special_char_roundtrips() {
        let user = "@é/%+#:example.org:8448";
        let room = "!é/%+#:example.org:8448";
        let event = "$é/%+#:example.org";
        let user_key = user_ref(user, None).unwrap().id;
        let room_key = room_ref(room).unwrap().id;
        let event_key = event_key(event).unwrap();
        let message_key = message_key(event).unwrap();
        assert_eq!(
            user_key
                .components()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [user]
        );
        assert_eq!(
            room_key
                .components()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [room]
        );
        assert_eq!(
            event_key
                .components()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [event]
        );
        assert_eq!(
            message_key
                .components()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [event]
        );
        assert_eq!(user_key.as_str().parse::<UserKey>().unwrap(), user_key);
        assert_eq!(room_key.as_str().parse::<RoomKey>().unwrap(), room_key);
        assert_eq!(event_key.as_str().parse::<EventKey>().unwrap(), event_key);
        assert_eq!(
            message_key.as_str().parse::<MessageKey>().unwrap(),
            message_key
        );
    }

    #[test]
    fn text_mentions_and_reply_fallback() {
        let raw = raw(
            "m.room.message",
            json!({"msgtype":"m.text", "body":"> <@bob:example.org> old\n> second\n\nhello Alice",
            "m.relates_to":{"m.in_reply_to":{"event_id":"$original"}},
            "m.mentions":{"room":true,"user_ids":["@alice:example.org", "bad", "@alice:example.org"]},
            "format":"org.matrix.custom.html", "formatted_body":"<mx-reply><a href='https://matrix.to/#/@bob:example.org'>old</a></mx-reply><A class='pill' HREF=\"https://matrix.to/#/%40%C3%A9%2F%25%2B%23%3Aexample.org?via=example.org&amp;x=1\">É</A>"}),
        );
        let message = message(&raw);
        assert_eq!(message.text, "hello Alice");
        assert_eq!(message.sent_at_unix_ms, 42);
        assert_eq!(message.author.automation, Automation::Unknown);
        assert_eq!(
            message.mentions,
            [
                user_ref("@alice:example.org", None).unwrap().id,
                user_ref("@é/%+#:example.org", None).unwrap().id
            ]
        );
        assert_eq!(
            message.reply_to.unwrap().id,
            message_key("$original").unwrap()
        );
    }

    #[test]
    fn ordinary_quotes_and_missing_fallback_separator_are_preserved() {
        for body in ["> ordinary quote\n\ntext", "> no separator\ntext"] {
            assert_eq!(
                message(&raw(
                    "m.room.message",
                    json!({"msgtype":"m.text","body":body})
                ))
                .text,
                body
            );
        }
        let body = "> no separator\ntext";
        assert_eq!(message(&raw("m.room.message", json!({"msgtype":"m.text","body":body,"m.relates_to":{"m.in_reply_to":{"event_id":"$original"}}}))).text, body);
    }

    #[test]
    fn edits_self_events_and_unknown_types_are_consumed() {
        assert!(decode(&raw("m.room.message", json!({"msgtype":"m.text","body":"* changed","m.relates_to":{"rel_type":"m.replace","event_id":"$original"},"m.new_content":{"body":"changed","msgtype":"m.text"}})), true).unwrap().is_none());
        for kind in ["m.room.message", "m.reaction", "m.room.member"] {
            let mut own = raw(kind, json!({}));
            own["sender"] = json!("@bot:example.org");
            assert!(decode(&own, true).unwrap().is_none());
        }
        assert!(decode(&json!({"type":"m.room.topic"}), true)
            .unwrap()
            .is_none());
        assert!(
            decode(&raw("m.room.message", json!({"msgtype":"m.file"})), true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn annotation_reaction_keeps_enrichment_unset() {
        let raw = raw(
            "m.reaction",
            json!({"m.relates_to":{"rel_type":"m.annotation","event_id":"$target","key":"👍🏽"}}),
        );
        let ChatEvent::ReactionAdded(reaction) = decode(&raw, true).unwrap().unwrap() else {
            panic!("not reaction")
        };
        assert_eq!(reaction.target.id, message_key("$target").unwrap());
        assert!(reaction.target.author.is_none());
        assert!(reaction.original_text.is_none());
        assert!(matches!(&reaction.emoji, Emoji::Unicode(emoji) if emoji == "👍🏽"));
    }

    #[test]
    fn images_require_analysis_and_native_mxc_media() {
        let raw = raw(
            "m.room.message",
            json!({"msgtype":"m.image","body":"photo.png","url":"mxc://example.org/opaque%+id","info":{"mimetype":"image/png","size":123}}),
        );
        assert!(decode(&raw, false).unwrap().is_none());
        let message = message(&raw);
        assert_eq!(message.attachments.len(), 1);
        let attachment = &message.attachments[0];
        assert_eq!(attachment.kind, MediaKind::Image);
        assert_eq!(attachment.size, Some(123));
        assert_eq!(attachment.mime.as_deref(), Some("image/png"));
        assert!(
            matches!(&attachment.source, MediaSource::Native { connection, id } if connection.as_str() == "matrix-main" && id == "mxc://example.org/opaque%+id")
        );
        for url in [
            "https://example.org/image.png",
            "mxc://example.org/",
            "mxc:///id",
            "mxc://example.org/id?token=secret",
        ] {
            let mut invalid = raw.clone();
            invalid["content"]["url"] = json!(url);
            assert!(decode(&invalid, true).is_err());
        }
        let mut encrypted = raw;
        encrypted["content"]["file"] = json!({"url":"mxc://example.org/id"});
        assert!(matches!(
            decode(&encrypted, true),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn joins_only_new_memberships_and_never_own_identity() {
        let mut raw = raw(
            "m.room.member",
            json!({"membership":"join","displayname":"Alice"}),
        );
        raw["state_key"] = json!("@alice:example.org");
        let ChatEvent::MemberJoined { user, .. } = decode(&raw, true).unwrap().unwrap() else {
            panic!("not join")
        };
        assert_eq!(user.display_name, "Alice");
        assert_eq!(user.automation, Automation::Unknown);
        raw["unsigned"] = json!({"prev_content":{"membership":"join"}});
        assert!(decode(&raw, true).unwrap().is_none());
        raw["unsigned"] = json!({"prev_content":{"membership":"invite"}});
        assert!(decode(&raw, true).unwrap().is_some());
        raw["state_key"] = json!("@bot:example.org");
        assert!(decode(&raw, true).unwrap().is_none());
        raw["content"]["membership"] = json!("leave");
        assert!(decode(&raw, true).unwrap().is_none());
    }

    #[test]
    fn encrypted_events_fail_closed_even_without_metadata() {
        assert!(matches!(
            decode(&json!({"type":"m.room.encrypted"}), true),
            Err(PlatformError::Unsupported(
                "encrypted rooms not supported yet"
            ))
        ));
    }

    #[test]
    fn required_metadata_and_reply_ids_fail_safely() {
        let raw = raw("m.room.message", json!({"msgtype":"m.text","body":"hello"}));
        for field in ["event_id", "sender", "origin_server_ts"] {
            let mut missing = raw.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(decode(&missing, true).is_err());
        }
        for timestamp in [json!(-1), json!(u64::MAX), json!(1.5), json!("42")] {
            let mut invalid = raw.clone();
            invalid["origin_server_ts"] = timestamp;
            assert!(decode(&invalid, true).is_err());
        }
        for id in ["", "$", "$bad\nID", "not-an-event"] {
            let mut invalid = raw.clone();
            invalid["event_id"] = json!(id);
            assert!(decode(&invalid, true).is_err());
        }
        let mut mismatch = raw.clone();
        mismatch["room_id"] = json!("!other:example.org");
        assert!(decode(&mismatch, true).is_err());
        let mut invalid_reply = raw;
        invalid_reply["content"]["m.relates_to"] = json!({"m.in_reply_to":{"event_id":"bad"}});
        assert!(decode(&invalid_reply, true).is_err());
    }

    #[test]
    fn html_mentions_ignore_untrusted_links_and_decode_entities() {
        let message = message(&raw(
            "m.room.message",
            json!({"msgtype":"m.text","body":"hello", "format":"org.matrix.custom.html",
            "formatted_body":"<a href='https://evil.example/#/@evil:example.org'>bad</a><a href='https://matrix.to/#/!room:example.org'>room</a><a href='https://matrix.to/#/%ZZ'>invalid</a><a href='https://matrix.to/#/&#64;bob:example.org'>Bob</a><a href='https://matrix.to/#/@bob:example.org'>duplicate</a>"}),
        ));
        assert_eq!(
            message.mentions,
            [user_ref("@bob:example.org", None).unwrap().id]
        );
    }
}

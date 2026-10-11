//! Matrix has no private thought-reveal UI: hidden reasoning never enters payloads.
use super::events::valid_native;
use crate::platform::{
    capabilities::{CountUnit, OutputLimits},
    interface::{PlatformError, PlatformResult},
    outgoing::{OutgoingMessage, PresentationPayload, ThoughtPresentation},
    types::{KeyError, PlatformKind, UserKey},
};
use serde_json::{json, Value};
use std::borrow::Cow;

pub(crate) const OUTPUT_LIMITS: OutputLimits = OutputLimits {
    text: 16_000,
    count_unit: CountUnit::EncodedBytes,
};

pub(crate) fn render_stream(
    mode: ThoughtPresentation,
    thought: &str,
    text: &str,
    done: bool,
) -> Vec<String> {
    let reply = crate::parse_reply(crate::stream_slice(text, done), OUTPUT_LIMITS);
    if mode != ThoughtPresentation::Show || thought.trim().is_empty() {
        return reply.lines;
    }
    let mut rendered = render_final(mode, thought, &[]);
    rendered.extend(reply.lines);
    rendered
}

pub(crate) fn render_final(
    mode: ThoughtPresentation,
    thought: &str,
    lines: &[String],
) -> Vec<String> {
    let mut rendered = Vec::new();
    if mode == ThoughtPresentation::Show && !thought.trim().is_empty() {
        let labelled = format!("{}:\n{thought}", crate::strings::t("thinking.title"));
        rendered.extend(crate::reply_chunks(&labelled, OUTPUT_LIMITS));
    }
    for line in lines {
        if !line.trim().is_empty() {
            rendered.extend(crate::reply_chunks(line, OUTPUT_LIMITS));
        }
    }
    rendered
}

fn native_component(
    platform: PlatformKind,
    mut components: impl Iterator<Item = Result<String, KeyError>>,
    sigil: char,
) -> PlatformResult<String> {
    if platform != PlatformKind::Matrix {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    let native = components
        .next()
        .ok_or(PlatformError::InvalidIdentity(KeyError))??;
    if components.next().is_some() || !valid_native(&native, sigil) {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    Ok(native)
}

fn native_user(user: &UserKey) -> PlatformResult<String> {
    native_component(user.platform(), user.components(), '@')
}

fn allow_user(users: &mut Vec<String>, user: &UserKey) -> PlatformResult<()> {
    let native = native_user(user)?;
    if !users.contains(&native) {
        users.push(native);
    }
    Ok(())
}

/// Build unformatted text with explicit, default-deny mention metadata. Uploads and
/// edit relations belong to transport; no text scanning can authorize a mention.
pub(crate) fn content(body: &OutgoingMessage<'_>) -> PlatformResult<Value> {
    if matches!(body.presentation, Some(PresentationPayload::ThoughtReveal)) {
        return Err(PlatformError::Unsupported("private thought reveal"));
    }
    let mut users = Vec::with_capacity(body.mentions.users.len());
    for user in body.mentions.users {
        allow_user(&mut users, user)?;
    }
    let mut text = Cow::Borrowed(body.text.as_ref());
    if let Some(user) = body.mentions.prefix_user {
        let native = native_user(user)?;
        text = Cow::Owned(format!("{native} {text}"));
        if !users.contains(&native) {
            users.push(native);
        }
    }
    if let Some(PresentationPayload::Card { title, description }) = body.presentation {
        text = Cow::Owned(if text.is_empty() {
            format!("{title}\n{description}")
        } else {
            format!("{text}\n{title}\n{description}")
        });
    }
    if OUTPUT_LIMITS.count(&text) > OUTPUT_LIMITS.text {
        return Err(PlatformError::Unsupported(
            "Matrix message exceeds text limit",
        ));
    }
    let mut content = json!({
        "msgtype": "m.text",
        "body": text,
        "m.mentions": { "room": false, "user_ids": users },
    });
    if let Some(reply) = body.reply_to {
        // Room equality is checked by transport, which knows the destination room.
        native_component(reply.room.platform(), reply.room.components(), '!')?;
        let native = native_component(reply.id.platform(), reply.id.components(), '$')?;
        content["m.relates_to"] = json!({ "m.in_reply_to": { "event_id": native } });
        if body.mentions.reply_user {
            if let Some(author) = &reply.author {
                let native = native_user(author)?;
                let users = content["m.mentions"]["user_ids"]
                    .as_array_mut()
                    .expect("built array");
                if !users
                    .iter()
                    .any(|user| user.as_str() == Some(native.as_str()))
                {
                    users.push(Value::String(native));
                }
            }
        }
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{
        matrix::events::{message_key, room_ref, user_ref},
        outgoing::MentionPolicy,
        types::{MessageRef, RoomKey},
    };

    #[test]
    fn hidden_silent_and_off_never_leak_thoughts() {
        for mode in [
            ThoughtPresentation::Hide,
            ThoughtPresentation::Silent,
            ThoughtPresentation::Off,
        ] {
            assert!(render_stream(mode, "private secret", "", false).is_empty());
            assert_eq!(
                render_stream(mode, "private secret", "public reply", true),
                ["public reply"]
            );
            assert_eq!(
                render_final(mode, "private secret", &["public reply".into()]),
                ["public reply"]
            );
        }
    }

    #[test]
    fn shown_thought_has_localized_explicit_label() {
        let rendered = render_stream(ThoughtPresentation::Show, "reasoning", "reply", true);
        assert_eq!(
            rendered,
            [
                format!("{}:\nreasoning", crate::strings::t("thinking.title")),
                "reply".to_owned()
            ]
        );
        assert_eq!(
            render_final(ThoughtPresentation::Show, " \n ", &["reply".into()]),
            ["reply"]
        );
    }

    #[test]
    fn stream_uses_shared_line_protocol() {
        assert_eq!(
            render_stream(ThoughtPresentation::Off, "", "hello\nincomplete", false),
            ["hello"]
        );
        assert_eq!(
            render_stream(ThoughtPresentation::Off, "", "hello\nincomplete", true),
            ["hello", "incomplete"]
        );
        assert_eq!(
            render_stream(ThoughtPresentation::Off, "", "hello\ntepki: 👍", true),
            ["hello"]
        );
        assert!(render_stream(ThoughtPresentation::Off, "", "-", true).is_empty());
    }

    #[test]
    fn unicode_chunks_preserve_every_byte_and_fit_encoded_limit() {
        let text = "🦀e\u{301}界üğ ".repeat(4000);
        let chunks = render_final(ThoughtPresentation::Off, "", std::slice::from_ref(&text));
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
        assert!(chunks
            .iter()
            .all(|chunk| !chunk.is_empty() && chunk.len() <= 16_000));
        let thought = "界🦀".repeat(4000);
        let chunks = render_final(ThoughtPresentation::Show, &thought, &[]);
        assert_eq!(
            chunks.concat(),
            format!("{}:\n{thought}", crate::strings::t("thinking.title"))
        );
        assert!(chunks.iter().all(|chunk| chunk.len() <= 16_000));
        let exact = "🦀".repeat(4000);
        assert_eq!(
            render_final(ThoughtPresentation::Off, "", std::slice::from_ref(&exact)),
            [exact]
        );
    }

    #[test]
    fn text_does_not_authorize_users_or_room_mentions() {
        let content = content(&OutgoingMessage::text(
            "@room @alice:example.org https://matrix.to/#/@alice:example.org",
        ))
        .unwrap();
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content["m.mentions"], json!({"room":false,"user_ids":[]}));
        assert!(content.get("formatted_body").is_none());
        assert!(content.get("m.relates_to").is_none());
    }

    #[test]
    fn authorized_users_are_native_and_deduplicated() {
        let alice = user_ref("@é/%+#:example.org", None).unwrap().id;
        let bob = user_ref("@bob:example.org", None).unwrap().id;
        let users = [alice.clone(), alice.clone()];
        let mut body = OutgoingMessage::text("welcome");
        body.mentions = MentionPolicy {
            users: &users,
            prefix_user: Some(&bob),
            reply_user: false,
        };
        let content = content(&body).unwrap();
        assert_eq!(content["body"], "@bob:example.org welcome");
        assert_eq!(
            content["m.mentions"],
            json!({"room":false,"user_ids":["@é/%+#:example.org","@bob:example.org"]})
        );
    }

    #[test]
    fn structured_reply_never_implicitly_pings_author() {
        let author = user_ref("@alice:example.org", None).unwrap().id;
        let target = MessageRef {
            id: message_key("$original/+%:server").unwrap(),
            room: room_ref("!room:example.org").unwrap().id,
            author: Some(author.clone()),
        };
        let mut body = OutgoingMessage::text("reply");
        body.reply_to = Some(&target);
        let denied = content(&body).unwrap();
        assert_eq!(
            denied["m.relates_to"],
            json!({"m.in_reply_to":{"event_id":"$original/+%:server"}})
        );
        assert_eq!(denied["m.mentions"]["user_ids"], json!([]));
        body.mentions.reply_user = true;
        assert_eq!(
            content(&body).unwrap()["m.mentions"]["user_ids"],
            json!(["@alice:example.org"])
        );
        let users = [author];
        body.mentions.users = &users;
        assert_eq!(
            content(&body).unwrap()["m.mentions"]["user_ids"],
            json!(["@alice:example.org"])
        );
    }

    #[test]
    fn foreign_or_invalid_native_identities_are_rejected() {
        for user in [
            UserKey::discord(1),
            UserKey::new(PlatformKind::Matrix, &["not-a-user"]).unwrap(),
        ] {
            let users = [user];
            let mut body = OutgoingMessage::text("hello");
            body.mentions.users = &users;
            assert!(matches!(
                content(&body),
                Err(PlatformError::InvalidIdentity(_))
            ));
        }
        let target = MessageRef {
            id: message_key("$original").unwrap(),
            room: RoomKey::discord(2),
            author: None,
        };
        let mut body = OutgoingMessage::text("reply");
        body.reply_to = Some(&target);
        assert!(content(&body).is_err());
    }

    #[test]
    fn cards_are_plaintext_and_private_reveals_are_unsupported() {
        let mut body = OutgoingMessage::text("");
        body.presentation = Some(PresentationPayload::Card {
            title: "Title",
            description: "Description",
        });
        assert_eq!(content(&body).unwrap()["body"], "Title\nDescription");
        body.presentation = Some(PresentationPayload::ThoughtReveal);
        assert!(matches!(
            content(&body),
            Err(PlatformError::Unsupported("private thought reveal"))
        ));
    }

    #[test]
    fn serialization_enforces_byte_budget_including_prefix() {
        let exact = "🦀".repeat(4000);
        assert!(content(&OutgoingMessage::text(&exact)).is_ok());
        let oversized = format!("{exact}a");
        assert!(content(&OutgoingMessage::text(&oversized)).is_err());
        let user = user_ref("@alice:example.org", None).unwrap().id;
        let mut body = OutgoingMessage::text(&exact);
        body.mentions.prefix_user = Some(&user);
        assert!(content(&body).is_err());
    }
}

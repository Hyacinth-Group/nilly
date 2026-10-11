//! Slack mrkdwn presentation. Input stays raw until the transport escapes it once.
use crate::platform::{outgoing::*, PlatformError, PlatformResult};

pub(super) const RAW_LIMIT: usize = 3900;
pub(super) const ENCODED_LIMIT: usize = 4000;

fn escaped_width(character: char) -> usize {
    match character {
        '&' => 5,
        '<' | '>' => 4,
        _ => 1,
    }
}

/// Split raw UTF-8 without cutting a scalar or an eventual HTML entity.
/// Leave room for the adapter's deliberate welcome/reply mention prefix.
pub(super) fn split(text: &str) -> Vec<String> {
    const PREFIX_RESERVE: usize = 160;
    let mut result = Vec::new();
    let mut start = 0;
    let mut raw = 0;
    let mut encoded = 0;
    for (offset, character) in text.char_indices() {
        let width = escaped_width(character);
        if raw == RAW_LIMIT - PREFIX_RESERVE || encoded + width > ENCODED_LIMIT - PREFIX_RESERVE {
            result.push(text[start..offset].to_owned());
            start = offset;
            raw = 0;
            encoded = 0;
        }
        raw += 1;
        encoded += width;
    }
    if start < text.len() {
        result.push(text[start..].to_owned());
    }
    result
}

pub(super) fn stream(
    mode: ThoughtPresentation,
    thought: &str,
    text: &str,
    done: bool,
) -> Vec<String> {
    let visible = if done {
        text
    } else {
        text.rfind('\n').map_or("", |end| &text[..=end])
    };
    let mut result = if mode == ThoughtPresentation::Show && !thought.trim().is_empty() {
        split(thought)
    } else {
        Vec::new()
    };
    result.extend(split(visible));
    result
}

pub(super) fn layout(mode: ThoughtPresentation, thought: &str, lines: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    if mode == ThoughtPresentation::Show && !thought.trim().is_empty() {
        // No spoiler/reveal component exists in this adapter. Show means visible;
        // all other modes exclude the thought entirely, including while streaming.
        result.extend(split(thought));
    }
    for line in lines {
        result.extend(split(line));
    }
    result
}

fn escape_into(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

pub(super) fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    escape_into(&mut output, text);
    output
}

pub(super) fn outgoing_text(team: &str, body: &OutgoingMessage<'_>) -> PlatformResult<String> {
    let card = match body.presentation {
        Some(PresentationPayload::ThoughtReveal) => {
            return Err(PlatformError::Unsupported(
                "Slack thought reveal components",
            ));
        }
        Some(PresentationPayload::Card { title, description }) => Some((title, description)),
        None => None,
    };
    let extra_scalars = card.map_or(0, |(title, description)| {
        title.chars().count() + description.chars().count() + 1 + usize::from(!body.text.is_empty())
    });
    if body.text.chars().count() + extra_scalars > RAW_LIMIT {
        return Err(PlatformError::Unsupported("Slack raw message text limit"));
    }
    let extra_bytes = card.map_or(0, |(title, description)| {
        title.len() + description.len() + 2
    });
    let mut output = String::with_capacity(body.text.len() + extra_bytes);
    let allowed_users = body
        .mentions
        .users
        .iter()
        .map(|user| super::transport::native_user(team, user))
        .collect::<PlatformResult<Vec<_>>>()?;
    let reply_user = body
        .mentions
        .reply_user
        .then(|| body.reply_to.and_then(|target| target.author.as_ref()))
        .flatten()
        .filter(|user| Some(*user) != body.mentions.prefix_user);
    for user in [body.mentions.prefix_user, reply_user]
        .into_iter()
        .flatten()
    {
        let native = super::transport::native_user(team, user)?;
        output.push_str("<@");
        output.push_str(&native);
        output.push_str("> ");
    }
    // Allow only structured, explicitly permitted user IDs; channel/broadcast/
    // subteam/link tokens and untrusted user tokens are always escaped.
    let mut rest = body.text.as_ref();
    while let Some(start) = rest.find("<@") {
        escape_into(&mut output, &rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('>') else {
            break;
        };
        let candidate = &rest[2..end];
        let allowed = allowed_users.iter().any(|native| native == candidate);
        if allowed {
            output.push_str(&rest[..=end]);
        } else {
            escape_into(&mut output, &rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    escape_into(&mut output, rest);
    if let Some((title, description)) = card {
        if !body.text.is_empty() {
            output.push('\n');
        }
        escape_into(&mut output, title);
        output.push('\n');
        escape_into(&mut output, description);
    }
    if output.chars().count() > ENCODED_LIMIT {
        return Err(PlatformError::Unsupported(
            "Slack escaped message text limit",
        ));
    }
    Ok(output)
}

pub(super) fn message_payload(channel: &str, escaped_text: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        "text": escaped_text,
        "mrkdwn": true,
        "parse": "none",
        "link_names": false,
        "unfurl_links": false,
        "unfurl_media": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::types::*;
    use std::borrow::Cow;

    #[test]
    fn outgoing_denies_broadcast_links_and_native_markup_by_default() {
        let text =
            "& <@U123> <!here> <!channel> <!everyone> <#C123> <https://example.test|link> @here";
        let escaped = outgoing_text("T123", &OutgoingMessage::text(text)).unwrap();
        assert_eq!(escaped, escape(text));
        assert!(!escaped.contains('<'));
        let payload = message_payload("C123", &escaped);
        assert_eq!(payload["parse"], "none");
        assert_eq!(payload["link_names"], false);
        assert_eq!(payload["unfurl_links"], false);
        assert_eq!(payload["unfurl_media"], false);
    }

    #[test]
    fn cards_fall_back_to_bounded_escaped_labelled_text() {
        let mut body = OutgoingMessage::text("");
        body.presentation = Some(PresentationPayload::Card {
            title: "Trace & info",
            description: "<!here> detail",
        });
        assert_eq!(
            outgoing_text("T123", &body).unwrap(),
            "Trace &amp; info\n&lt;!here&gt; detail"
        );
        body.presentation = Some(PresentationPayload::ThoughtReveal);
        assert!(matches!(
            outgoing_text("T123", &body),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn explicit_mentions_cannot_authorize_broadcast_or_foreign_team() {
        let user = UserKey::new(PlatformKind::Slack, &["T123", "U123"]).unwrap();
        let mut body = OutgoingMessage::text("<@U123> <@U999> <!here>");
        body.mentions.users = std::slice::from_ref(&user);
        body.mentions.prefix_user = Some(&user);
        assert_eq!(
            outgoing_text("T123", &body).unwrap(),
            "<@U123> <@U123> &lt;@U999&gt; &lt;!here&gt;"
        );
        assert!(outgoing_text("T999", &body).is_err());
    }

    #[test]
    fn split_budgets_unicode_and_escaped_expansion_without_loss() {
        for input in [
            "💀".repeat(12000),
            "&<>".repeat(5000),
            "normal\n".repeat(1000),
        ] {
            let chunks = split(&input);
            assert_eq!(chunks.concat(), input);
            for chunk in chunks {
                assert!(chunk.chars().count() <= RAW_LIMIT);
                assert!(escape(&chunk).chars().count() <= ENCODED_LIMIT);
                assert!(outgoing_text("T123", &OutgoingMessage::text(&chunk)).is_ok());
            }
        }
        let mut body = OutgoingMessage::text("");
        body.text = Cow::Owned("&".repeat(1000));
        assert!(outgoing_text("T123", &body).is_err());
    }

    #[test]
    fn thoughts_only_appear_in_show_mode() {
        for mode in [
            ThoughtPresentation::Hide,
            ThoughtPresentation::Silent,
            ThoughtPresentation::Off,
        ] {
            assert_eq!(layout(mode, "secret", &["reply".into()]), vec!["reply"]);
            assert!(stream(mode, "secret", "", false).is_empty());
        }
        assert_eq!(
            layout(ThoughtPresentation::Show, "thought", &["reply".into()]),
            vec!["thought", "reply"]
        );
    }
}

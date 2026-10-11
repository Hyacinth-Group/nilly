// named ChatMessage, not Message: `use serenity::all::*` already brings in Discord's own
// Message type, and this crate talks about both in the same file constantly
/// One turn of the AI chat history sent to the provider (OpenRouter/Mistral), not a Discord
/// message. Holds:
/// - `role`: `"user"` or `"assistant"` (matches the OpenAI-style chat/completions schema).
/// - `content`: the message text.
/// - `image`: URL of an attached image, if any (see field comment below).
///
/// Built by `user`/`user_with_image`/`assistant`; turned into the request JSON by
/// `message_json`. Stored in `Chat.history` (`types_chat_state.rs`) and passed around as
/// `&[ChatMessage]` by `Bot::ask`/`ask_split`/`ask_raw_stream`/`generate`/`generate_stream`.
#[derive(Serialize, Clone)]
struct ChatMessage {
    role: &'static str,
    content: String,
    // URL of the attached image; the request body is built by hand, so this never goes
    // through serialization. Only ever set on the latest user message: a Discord CDN link
    // is short-lived, and resending the old image on every turn would waste tokens for
    // nothing.
    #[serde(skip)]
    image: Option<String>,
}

/// Builds a plain user turn (no image).
fn user(text: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role: "user",
        content: text.into(),
        image: None,
    }
}

// a user message with an image attached: both the text and the image go to the model
/// Builds a user turn with an attached image.
fn user_with_image(text: impl Into<String>, url: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role: "user",
        content: text.into(),
        image: Some(url.into()),
    }
}

/// Builds an assistant turn (the bot's own prior reply, for seeding chat history).
fn assistant(text: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role: "assistant",
        content: text.into(),
        image: None,
    }
}

// turns a message into an OpenAI-compatible request block. With an image, content is a
// multi-part array instead of plain text; the shape matches the image commenter in
// agents.rs, since providers understand this same format
/// Serializes one `ChatMessage` into the JSON shape a chat/completions request expects.
/// Image turns use text/image_url content blocks; other turns use a plain content string.
fn message_json(m: &ChatMessage) -> serde_json::Value {
    match &m.image {
        None => serde_json::json!({ "role": m.role, "content": m.content }),
        Some(url) => serde_json::json!({
            "role": m.role,
            "content": [
                { "type": "text", "text": m.content },
                { "type": "image_url", "image_url": { "url": url } }
            ]
        }),
    }
}

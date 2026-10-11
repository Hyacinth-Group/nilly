//! Telegram Bot API polling adapter. Native tokens and file URLs remain private.
mod client;
pub(crate) mod events;
pub(crate) mod runner;

use crate::platform::{
    capabilities::*, outgoing::*, types::*, Platform, PlatformError, PlatformResult,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use client::TelegramClient;
use events::{message_key, room_key, user_ref, User};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

const IMAGE_LIMIT: usize = 20 * 1024 * 1024;
const PHOTO_UPLOAD_LIMIT: usize = 10 * 1024 * 1024;
const OUTPUT: OutputLimits = OutputLimits {
    text: 4096,
    count_unit: CountUnit::UnicodeScalars,
};

#[derive(Clone)]
struct KnownRoom {
    room: RoomRef,
    capabilities: Capabilities,
}
pub struct TelegramAdapter {
    connection: ConnectionId,
    account: UserRef,
    native_account: User,
    client: Arc<TelegramClient>,
    rooms: Mutex<HashMap<RoomKey, KnownRoom>>,
}
fn photo_permission(
    kind: RoomKind,
    status: &str,
    member: &Value,
    chat: &Value,
    send: bool,
) -> bool {
    send && match status {
        "creator" | "administrator" => true,
        "member" if kind == RoomKind::Direct => true,
        "member" => {
            chat.pointer("/permissions/can_send_photos")
                .and_then(Value::as_bool)
                == Some(true)
        }
        "restricted" => member.get("can_send_photos").and_then(Value::as_bool) == Some(true),
        _ => false,
    }
}
impl TelegramAdapter {
    pub(crate) fn new(
        connection: ConnectionId,
        account: User,
        client: Arc<TelegramClient>,
    ) -> Self {
        Self {
            connection,
            account: user_ref(&account),
            native_account: account,
            client,
            rooms: Mutex::new(HashMap::new()),
        }
    }
    pub(crate) async fn discover(&self, room: &RoomRef) -> PlatformResult<()> {
        let id = chat_id(&room.id)?;
        let member: Value = if room.kind == RoomKind::Direct {
            json!({"status":"member"})
        } else {
            self.client
                .call(
                    "getChatMember",
                    &json!({"chat_id":id,"user_id":self.native_account.id}),
                )
                .await?
        };
        let status = member.get("status").and_then(Value::as_str).unwrap_or("");
        let admin = matches!(status, "creator" | "administrator");
        let info: Value = self.client.call("getChat", &json!({"chat_id":id})).await?;
        let send = match status {
            "left" | "kicked" => false,
            "restricted" => {
                member.get("is_member").and_then(Value::as_bool) == Some(true)
                    && member.get("can_send_messages").and_then(Value::as_bool) == Some(true)
            }
            "member" | "creator" | "administrator" => {
                admin
                    || room.kind == RoomKind::Direct
                    || info
                        .pointer("/permissions/can_send_messages")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
            }
            _ => false,
        };
        let ordinary = ordinary_visibility(
            room.kind,
            admin,
            self.native_account.can_read_all_group_messages,
        );
        let mut capabilities = Capabilities::unavailable();
        capabilities.visibility = if ordinary {
            MessageVisibility::Ordinary
        } else {
            MessageVisibility::MentionsOnly
        };
        capabilities.replies = true;
        capabilities.threads = info
            .get("is_forum")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        capabilities.send = send;
        capabilities.send_attachments = photo_permission(room.kind, status, &member, &info, send);
        // Reactions must be explicitly listed by getChat; do not assume all emoji are usable.
        capabilities.react = send
            && info
                .get("available_reactions")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|reaction| {
                        reaction.get("type").and_then(Value::as_str) == Some("emoji")
                    })
                });
        capabilities.edit = send;
        capabilities.delete = send;
        capabilities.typing = send;
        capabilities.native_commands = true;
        capabilities.private_results = room.kind == RoomKind::Direct;
        capabilities.output = OUTPUT;
        capabilities.edit_cadence = Duration::from_millis(1200);
        capabilities.upload_bytes = Some(PHOTO_UPLOAD_LIMIT as u64);
        if !ordinary {
            log::warn!(
                "Telegram [{id}]: {}",
                crate::strings::t("telegram.privacy_warning")
            );
        }
        self.rooms.lock().unwrap_or_else(|e| e.into_inner()).insert(
            room.id.clone(),
            KnownRoom {
                room: room.clone(),
                capabilities,
            },
        );
        Ok(())
    }
    fn require_send(&self, room: &RoomRef) -> PlatformResult<()> {
        if !self.capabilities(room).send {
            return Err(PlatformError::PermissionDenied);
        }
        Ok(())
    }
    pub(crate) async fn private_text(&self, user: i64, text: &str) -> PlatformResult<()> {
        for part in crate::reply_chunks(&safe_text(text), OUTPUT) {
            self.client.call::<Value>("sendMessage", &json!({"chat_id":user,"text":part,"entities":[],"link_preview_options":{"is_disabled":true}})).await?;
        }
        Ok(())
    }
}
fn ordinary_visibility(kind: RoomKind, admin: bool, privacy_disabled: Option<bool>) -> bool {
    kind == RoomKind::Direct || admin || privacy_disabled == Some(true)
}
fn chat_id(key: &RoomKey) -> PlatformResult<i64> {
    if key.platform() != PlatformKind::Telegram {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    key.components()
        .next()
        .ok_or(KeyError)??
        .parse()
        .map_err(|_| PlatformError::InvalidIdentity(KeyError))
}
fn native_message(target: &MessageRef) -> PlatformResult<i64> {
    if target.id.platform() != PlatformKind::Telegram {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    let mut components = target.id.components();
    if components.next().ok_or(KeyError)?? != chat_id(&target.room)?.to_string() {
        return Err(PlatformError::InvalidIdentity(KeyError));
    }
    components
        .next()
        .ok_or(KeyError)??
        .parse()
        .map_err(|_| PlatformError::InvalidIdentity(KeyError))
}
fn safe_text(text: &str) -> String {
    // Telegram detects bare @usernames even without parse_mode/entities. Break them
    // with an invisible separator; explicit allowed text_mention entities are separate.
    let mut safe = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        safe.push(c);
        if c == '@' && chars.peek() != Some(&'\u{200b}') {
            safe.push('\u{200b}');
        }
    }
    safe
}
fn payload(room: &RoomRef, body: &OutgoingMessage<'_>) -> PlatformResult<Value> {
    let mut text = safe_text(&body.text);
    if let Some(PresentationPayload::Card { title, description }) = body.presentation {
        text = format!("{}\n{}", safe_text(title), safe_text(description));
    }
    let mut entities = Vec::new();
    if let Some(user) = body
        .mentions
        .prefix_user
        .filter(|user| body.mentions.users.contains(user))
    {
        if user.platform() != PlatformKind::Telegram {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        let id: i64 = user
            .components()
            .next()
            .ok_or(KeyError)??
            .parse()
            .map_err(|_| PlatformError::InvalidIdentity(KeyError))?;
        text.insert_str(0, "@user ");
        entities.push(json!({"type":"text_mention","offset":0,"length":5,"user":{"id":id,"is_bot":false,"first_name":"user"}}));
    }
    if OUTPUT.count(&text) > OUTPUT.text {
        return Err(PlatformError::Transport(
            "Telegram text exceeds 4096 characters".into(),
        ));
    }
    let mut value = json!({"chat_id":chat_id(&room.id)?,"text":text,"entities":entities,"link_preview_options":{"is_disabled":true}});
    if let Some(thread) = &room.thread {
        if thread.platform() != PlatformKind::Telegram {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        let mut components = thread.components();
        if components.next().ok_or(KeyError)?? != chat_id(&room.id)?.to_string() {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        let id: i64 = components
            .next()
            .ok_or(KeyError)??
            .parse()
            .map_err(|_| PlatformError::InvalidIdentity(KeyError))?;
        value["message_thread_id"] = json!(id);
    }
    if let Some(reply) = body.reply_to {
        if reply.room != room.id {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        value["reply_parameters"] =
            json!({"message_id":native_message(reply)?,"allow_sending_without_reply":true});
    }
    Ok(value)
}
#[derive(Deserialize)]
struct SentMessage {
    message_id: i64,
    chat: SentChat,
}
#[derive(Deserialize)]
struct SentChat {
    id: i64,
}
impl Platform for TelegramAdapter {
    fn connection_id(&self) -> &ConnectionId {
        &self.connection
    }
    fn self_user(&self) -> &UserRef {
        &self.account
    }
    fn connection_warnings(&self) -> Vec<ConnectionWarning> {
        if self.native_account.can_read_all_group_messages == Some(true) {
            Vec::new()
        } else {
            vec![ConnectionWarning::TelegramPrivacyLimited]
        }
    }
    fn capabilities(&self, room: &RoomRef) -> Capabilities {
        self.rooms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&room.id)
            .map(|known| known.capabilities.clone())
            .unwrap_or_else(Capabilities::unavailable)
    }
    fn render_stream(
        &self,
        _room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String> {
        let reply = crate::parse_reply(crate::stream_slice(text, done), OUTPUT);
        let mut output = Vec::new();
        // Thought text is explicit presentation, not part of the reply protocol.
        if mode == ThoughtPresentation::Show && !thought.is_empty() {
            output.extend(crate::reply_chunks(&safe_text(thought), OUTPUT));
        }
        for line in &reply.lines {
            output.extend(crate::reply_chunks(&safe_text(line), OUTPUT));
        }
        output
    }
    fn render_final(
        &self,
        room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String> {
        let mut output = Vec::new();
        if mode == ThoughtPresentation::Show && !thought.is_empty() {
            output.extend(self.render_stream(room, mode, thought, "", true));
        }
        for line in lines {
            output.extend(crate::reply_chunks(&safe_text(line), OUTPUT));
        }
        output
    }
    async fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<MessageReceipt> {
        self.require_send(room)?;
        let mut value = payload(room, body)?;
        let sent: SentMessage = if body.attachments.is_empty() {
            self.client.call("sendMessage", &value).await?
        } else {
            if body.attachments.len() != 1 {
                return Err(PlatformError::Unsupported("multiple Telegram uploads"));
            }
            let (filename, data, mime) = match &body.attachments[0] {
                OutgoingAttachment::LocalPath(path) => {
                    let meta = tokio::fs::metadata(path)
                        .await
                        .map_err(|_| PlatformError::NotFound)?;
                    if meta.len() > PHOTO_UPLOAD_LIMIT as u64 {
                        return Err(PlatformError::Transport(
                            "Telegram photo exceeds upload limit".into(),
                        ));
                    }
                    let data = tokio::fs::read(path)
                        .await
                        .map_err(|_| PlatformError::NotFound)?;
                    (
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("photo.jpg")
                            .to_owned(),
                        data,
                        None,
                    )
                }
                OutgoingAttachment::Bytes {
                    filename,
                    data,
                    mime,
                } => (filename.to_string(), data.to_vec(), *mime),
            };
            if data.len() > PHOTO_UPLOAD_LIMIT {
                return Err(PlatformError::Transport(
                    "Telegram photo exceeds upload limit".into(),
                ));
            }
            let mut part = reqwest::multipart::Part::bytes(data).file_name(filename);
            if let Some(mime) = mime {
                part = part
                    .mime_str(mime)
                    .map_err(|_| PlatformError::Transport("invalid upload MIME type".into()))?;
            }
            let text = value["text"].take();
            if text
                .as_str()
                .is_some_and(|text| text.chars().count() > 1024)
            {
                return Err(PlatformError::Transport(
                    "Telegram photo caption exceeds 1024 characters".into(),
                ));
            }
            value["caption"] = text;
            value["caption_entities"] = value["entities"].take();
            let mut form = reqwest::multipart::Form::new().part("photo", part);
            for (key, field) in value.as_object().ok_or(KeyError)? {
                if key == "text" || key == "entities" || key == "link_preview_options" {
                    continue;
                }
                let text = field
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| field.to_string());
                form = form.text(key.clone(), text);
            }
            self.client.upload("sendPhoto", form).await?
        };
        let accepted = value
            .get("caption")
            .or_else(|| value.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Ok(MessageReceipt {
            connection: self.connection.clone(),
            message: MessageRef {
                id: message_key(sent.chat.id, sent.message_id),
                room: room_key(sent.chat.id),
                author: Some(self.account.id.clone()),
            },
            room: room.clone(),
            last_accepted_text: accepted,
            version: 0,
        })
    }
    async fn edit<'a>(
        &'a self,
        sent: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<()> {
        self.require_send(&sent.room)?;
        if !body.attachments.is_empty() {
            return Err(PlatformError::Unsupported("Telegram media editing"));
        }
        let mut value = payload(&sent.room, body)?;
        value["message_id"] = json!(native_message(&sent.message)?);
        self.client.call::<Value>("editMessageText", &value).await?;
        Ok(())
    }
    async fn delete<'a>(&'a self, sent: &'a MessageReceipt) -> PlatformResult<()> {
        self.client.call::<Value>("deleteMessage", &json!({"chat_id":chat_id(&sent.room.id)?,"message_id":native_message(&sent.message)?})).await?;
        Ok(())
    }
    async fn react<'a>(&'a self, target: &'a MessageRef, emoji: &'a Emoji) -> PlatformResult<()> {
        let Emoji::Unicode(emoji) = emoji else {
            return Err(PlatformError::Unsupported("Telegram custom reaction"));
        };
        let chat = chat_id(&target.room)?;
        let info: Value = self
            .client
            .call("getChat", &json!({"chat_id":chat}))
            .await?;
        if !info
            .get("available_reactions")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|r| {
                    r.get("type").and_then(Value::as_str) == Some("emoji")
                        && r.get("emoji").and_then(Value::as_str) == Some(emoji)
                })
            })
        {
            return Err(PlatformError::Unsupported(
                "Telegram reaction not allowed in this chat",
            ));
        }
        self.client.call::<Value>("setMessageReaction", &json!({"chat_id":chat,"message_id":native_message(target)?,"reaction":[{"type":"emoji","emoji":emoji}]})).await?;
        Ok(())
    }
    async fn typing<'a>(&'a self, room: &'a RoomRef) -> PlatformResult<()> {
        let mut value = payload(room, &OutgoingMessage::text(""))?;
        value["action"] = json!("typing");
        self.client.call::<Value>("sendChatAction", &value).await?;
        Ok(())
    }
    async fn set_nickname<'a>(
        &'a self,
        _target: NickTarget<'a>,
        _name: &'a str,
    ) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("Telegram per-group nickname"))
    }
    async fn fetch_history<'a>(
        &'a self,
        _room: &'a RoomRef,
        _page: HistoryRequest,
    ) -> PlatformResult<HistoryPage> {
        Err(PlatformError::Unsupported("Telegram group history"))
    }
    async fn list_rooms(&self) -> PlatformResult<Vec<RoomRef>> {
        Ok(self
            .rooms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|known| known.room.clone())
            .collect())
    }
    async fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> PlatformResult<ModelMedia> {
        if !crate::setting("IMAGE_ANALYSIS")
            .map(|v| v != "off")
            .unwrap_or(true)
        {
            return Err(PlatformError::Unsupported("image analysis is off"));
        }
        let MediaSource::Native { connection, id } = &attachment.source else {
            return Err(PlatformError::Unsupported("Telegram native media required"));
        };
        if connection != &self.connection || attachment.kind != MediaKind::Image {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        if attachment
            .size
            .is_some_and(|size| size > IMAGE_LIMIT as u64)
        {
            return Err(PlatformError::Transport(
                "Telegram image exceeds download limit".into(),
            ));
        }
        let file: Value = self.client.call("getFile", &json!({"file_id":id})).await?;
        let path = file
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or(PlatformError::NotFound)?;
        let bytes = self.client.download(path, IMAGE_LIMIT).await?;
        let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else {
            return Err(PlatformError::Unsupported(
                "unsupported Telegram image content",
            ));
        };
        Ok(ModelMedia::DataUrl(format!(
            "data:{mime};base64,{}",
            STANDARD.encode(bytes)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn photo_capability_uses_explicit_group_and_restricted_member_photo_rights() {
        let allowed = json!({"permissions":{"can_send_photos":true}});
        let denied = json!({"permissions":{"can_send_photos":false}});
        let member = json!({"can_send_photos":true});
        assert!(photo_permission(
            RoomKind::Group,
            "member",
            &member,
            &allowed,
            true
        ));
        assert!(!photo_permission(
            RoomKind::Group,
            "member",
            &member,
            &denied,
            true
        ));
        assert!(!photo_permission(
            RoomKind::Group,
            "member",
            &member,
            &json!({}),
            true
        ));
        assert!(photo_permission(
            RoomKind::Group,
            "restricted",
            &member,
            &denied,
            true
        ));
        assert!(!photo_permission(
            RoomKind::Group,
            "restricted",
            &json!({}),
            &allowed,
            true
        ));
        assert!(!photo_permission(
            RoomKind::Group,
            "restricted",
            &json!({"can_send_photos":false}),
            &allowed,
            true
        ));
    }

    #[test]
    fn photo_capability_requires_sending_even_for_direct_and_admin_rooms() {
        for (kind, status) in [
            (RoomKind::Direct, "member"),
            (RoomKind::Group, "administrator"),
            (RoomKind::Group, "creator"),
        ] {
            assert!(photo_permission(kind, status, &json!({}), &json!({}), true));
            assert!(!photo_permission(
                kind,
                status,
                &json!({}),
                &json!({}),
                false
            ));
        }
        assert!(!photo_permission(
            RoomKind::Group,
            "left",
            &json!({}),
            &json!({}),
            true
        ));
        assert!(!photo_permission(
            RoomKind::Group,
            "unknown",
            &json!({}),
            &json!({}),
            true
        ));
    }

    #[test]
    fn privacy_visibility_requires_disabled_mode_or_group_administration() {
        assert!(!ordinary_visibility(RoomKind::Group, false, None));
        assert!(!ordinary_visibility(RoomKind::Group, false, Some(false)));
        assert!(ordinary_visibility(RoomKind::Group, false, Some(true)));
        assert!(ordinary_visibility(RoomKind::Group, true, Some(false)));
        assert!(ordinary_visibility(RoomKind::Direct, false, None));
        assert!(crate::strings::t("telegram.privacy_warning").contains("/setprivacy"));
    }
    #[test]
    fn outgoing_mentions_default_deny_and_topic_reply_scope() {
        let room = RoomRef::from_key(room_key(-1001));
        let body = OutgoingMessage::text("hello @someone <b>plain</b>");
        let value = payload(&room, &body).unwrap();
        assert_eq!(value["text"], "hello @\u{200b}someone <b>plain</b>");
        assert_eq!(value["entities"], json!([]));
        assert!(value.get("parse_mode").is_none());
        let wrong = MessageRef {
            id: message_key(-1002, 1),
            room: room_key(-1002),
            author: None,
        };
        let mut body = body;
        body.reply_to = Some(&wrong);
        assert!(payload(&room, &body).is_err());
        let mut topic = room.clone();
        topic.thread = Some(events::thread_key(-1001, 7));
        let user = events::user_key(42);
        let users = [user.clone()];
        let reply = MessageRef {
            id: message_key(-1001, 9),
            room: room.id.clone(),
            author: Some(user.clone()),
        };
        body.reply_to = Some(&reply);
        body.mentions = MentionPolicy {
            users: &users,
            prefix_user: Some(&user),
            reply_user: false,
        };
        let value = payload(&topic, &body).unwrap();
        assert_eq!(value["message_thread_id"], 7);
        assert_eq!(value["reply_parameters"]["message_id"], 9);
        assert_eq!(value["entities"][0]["user"]["id"], 42);
        assert_eq!(safe_text("@\u{200b}already"), "@\u{200b}already");
    }
    #[test]
    fn telegram_limits_preserve_unicode_and_long_lines() {
        let text = "🦀".repeat(4097);
        let parts = crate::reply_chunks(&text, OUTPUT);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| part.chars().count() <= 4096));
        assert_eq!(parts.concat(), text);
    }

    fn stream_adapter() -> TelegramAdapter {
        TelegramAdapter::new(
            ConnectionId::new("telegram-stream-test"),
            User {
                id: 1,
                is_bot: true,
                first_name: "Bot".into(),
                last_name: None,
                username: None,
                can_read_all_group_messages: None,
            },
            Arc::new(TelegramClient::new("unused-test-token".into()).unwrap()),
        )
    }

    #[test]
    fn stream_never_publishes_reply_controls_or_their_partial_chunks() {
        let adapter = stream_adapter();
        let room = RoomRef::from_key(room_key(-1001));
        for mode in [
            ThoughtPresentation::Show,
            ThoughtPresentation::Hide,
            ThoughtPresentation::Silent,
            ThoughtPresentation::Off,
        ] {
            for protocol in ["-\n", "tepki: 👍\n", "tepki: 👍\n-\n", "-\ntepki: 👍\n"] {
                for (end, _) in protocol.char_indices() {
                    assert!(
                        adapter
                            .render_stream(&room, mode, "", &protocol[..end], false)
                            .is_empty(),
                        "published a partial control: {:?}",
                        &protocol[..end],
                    );
                    assert_eq!(
                        adapter.render_stream(
                            &room,
                            mode,
                            "",
                            &format!("hello\n{}", &protocol[..end]),
                            false,
                        ),
                        vec!["hello"],
                    );
                }
                for done in [false, true] {
                    assert!(adapter
                        .render_stream(&room, mode, "", protocol, done)
                        .is_empty());
                }
            }
            for done in [false, true] {
                assert_eq!(
                    adapter.render_stream(&room, mode, "", "hello\ntepki: 👍\n-\n", done,),
                    vec!["hello"],
                );
            }
            // A long reaction body crosses the half-line threshold but is still control.
            let reaction = format!("tepki: {}", "👍".repeat(crate::HALF_LINE_THRESHOLD));
            assert!(adapter
                .render_stream(&room, mode, "", &reaction, false)
                .is_empty());
        }
    }

    #[test]
    fn stream_uses_shared_half_line_rules_and_final_native_layout() {
        let adapter = stream_adapter();
        let room = RoomRef::from_key(room_key(-1001));
        let mode = ThoughtPresentation::Silent;
        let below = format!("hello\n{}", "ğ".repeat(crate::HALF_LINE_THRESHOLD - 1));
        let threshold = "ğ".repeat(crate::HALF_LINE_THRESHOLD);
        assert!(adapter
            .render_stream(&room, mode, "", "short", false)
            .is_empty());
        assert_eq!(
            adapter.render_stream(&room, mode, "", &below, false),
            vec!["hello"]
        );
        assert_eq!(
            adapter.render_stream(&room, mode, "", &format!("hello\n{threshold}"), false),
            vec!["hello".to_owned(), threshold],
        );
        assert_eq!(
            adapter.render_stream(&room, mode, "", "short", true),
            vec!["short"]
        );
        let lines = vec!["hello".into(), "short".into()];
        for mode in [
            ThoughtPresentation::Show,
            ThoughtPresentation::Hide,
            ThoughtPresentation::Silent,
            ThoughtPresentation::Off,
        ] {
            assert_eq!(
                adapter.render_stream(&room, mode, "thought", "hello\nshort", true),
                adapter.render_final(&room, mode, "thought", &lines),
            );
        }
    }

    #[test]
    fn stream_thought_presentation_is_separate_from_reply_control() {
        let adapter = stream_adapter();
        let room = RoomRef::from_key(room_key(-1001));
        let thought = "tepki: reflection\n-";
        for done in [false, true] {
            for mode in [
                ThoughtPresentation::Hide,
                ThoughtPresentation::Silent,
                ThoughtPresentation::Off,
            ] {
                assert!(adapter
                    .render_stream(&room, mode, thought, "tepki: 👍\n-\n", done)
                    .is_empty());
            }
            assert_eq!(
                adapter.render_stream(
                    &room,
                    ThoughtPresentation::Show,
                    thought,
                    "tepki: 👍\n-\n",
                    done
                ),
                vec![thought],
            );
        }
    }

    #[test]
    fn stream_native_chunks_preserve_unicode_and_safe_mentions() {
        let adapter = stream_adapter();
        let room = RoomRef::from_key(room_key(-1001));
        let text = "🦀".repeat(4097);
        for done in [false, true] {
            let chunks = adapter.render_stream(&room, ThoughtPresentation::Silent, "", &text, done);
            assert_eq!(chunks.concat(), safe_text(&text));
            assert!(chunks
                .iter()
                .all(|chunk| OUTPUT.count(chunk) <= OUTPUT.text));
            assert_eq!(
                adapter.render_stream(&room, ThoughtPresentation::Silent, "", "@someone", true),
                vec!["@\u{200b}someone"],
            );
            for chunk in chunks {
                assert_eq!(
                    payload(&room, &OutgoingMessage::text(&chunk)).unwrap()["text"],
                    chunk
                );
            }
        }
    }
}

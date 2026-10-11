//! Native Telegram payloads and pure, offline ingress normalization.
use crate::platform::types::*;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Update {
    pub(crate) update_id: i64,
    pub(crate) message: Option<Message>,
    pub(crate) edited_message: Option<Message>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct User {
    pub(crate) id: i64,
    pub(crate) is_bot: bool,
    pub(crate) first_name: String,
    pub(crate) last_name: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) can_read_all_group_messages: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Chat {
    pub(crate) id: i64,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) title: Option<String>,
    pub(crate) first_name: Option<String>,
    pub(crate) last_name: Option<String>,
    pub(crate) username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Message {
    pub(crate) message_id: i64,
    pub(crate) date: i64,
    pub(crate) chat: Chat,
    pub(crate) from: Option<User>,
    pub(crate) text: Option<String>,
    pub(crate) caption: Option<String>,
    #[serde(default)]
    pub(crate) entities: Vec<Entity>,
    #[serde(default)]
    pub(crate) caption_entities: Vec<Entity>,
    pub(crate) reply_to_message: Option<Box<Message>>,
    #[serde(default)]
    pub(crate) photo: Vec<Photo>,
    pub(crate) message_thread_id: Option<i64>,
    #[serde(default)]
    pub(crate) new_chat_members: Vec<User>,
    pub(crate) sender_chat: Option<Chat>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Entity {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) user: Option<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Photo {
    pub(crate) file_id: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) file_size: Option<u64>,
}

pub(crate) fn user_key(id: i64) -> UserKey {
    UserKey::new(PlatformKind::Telegram, &[&id.to_string()])
        .expect("Telegram user IDs are nonempty canonical components")
}

pub(crate) fn room_key(id: i64) -> RoomKey {
    RoomKey::new(PlatformKind::Telegram, &[&id.to_string()])
        .expect("Telegram chat IDs are nonempty canonical components")
}

pub(crate) fn message_key(chat_id: i64, message_id: i64) -> MessageKey {
    MessageKey::new(
        PlatformKind::Telegram,
        &[&chat_id.to_string(), &message_id.to_string()],
    )
    .expect("Telegram message IDs include their chat scope")
}

pub(crate) fn thread_key(chat_id: i64, thread_id: i64) -> ThreadKey {
    ThreadKey::new(
        PlatformKind::Telegram,
        &[&chat_id.to_string(), &thread_id.to_string()],
    )
    .expect("Telegram topic IDs include their chat scope")
}

fn full_name(first_name: &str, last_name: Option<&str>) -> String {
    match last_name.filter(|name| !name.is_empty()) {
        Some(last_name) => format!("{first_name} {last_name}"),
        None => first_name.to_owned(),
    }
}

pub(crate) fn user_ref(user: &User) -> UserRef {
    UserRef {
        id: user_key(user.id),
        display_name: full_name(&user.first_name, user.last_name.as_deref()),
        username: user.username.clone(),
        automation: if user.is_bot {
            Automation::Bot
        } else {
            Automation::Human
        },
        stability: IdentityStability::Persistent,
    }
}

pub(crate) fn room_ref(message: &Message) -> RoomRef {
    let chat = &message.chat;
    RoomRef {
        id: room_key(chat.id),
        space: None,
        kind: match chat.kind.as_str() {
            "private" => RoomKind::Direct,
            "channel" => RoomKind::Channel,
            _ => RoomKind::Group,
        },
        name: chat
            .title
            .clone()
            .or_else(|| {
                chat.first_name
                    .as_deref()
                    .map(|first| full_name(first, chat.last_name.as_deref()))
            })
            .or_else(|| chat.username.clone()),
        thread: message.message_thread_id.map(|id| thread_key(chat.id, id)),
    }
}

pub(crate) fn command_text(message: &Message) -> Option<&str> {
    message.text.as_deref()
}

/// Telegram entity offsets count UTF-16 units, not UTF-8 bytes or Rust chars.
/// Reject ranges that split a surrogate pair or extend beyond the text.
fn entity_text<'a>(text: &'a str, entity: &Entity) -> Option<&'a str> {
    let end = entity.offset.checked_add(entity.length)?;
    let mut units = 0;
    let mut start_byte = None;
    for (byte, character) in text.char_indices() {
        if units == entity.offset {
            start_byte = Some(byte);
        }
        if units == end {
            return text.get(start_byte?..byte);
        }
        units += character.len_utf16();
    }
    if units == entity.offset {
        start_byte = Some(text.len());
    }
    if units == end {
        text.get(start_byte?..text.len())
    } else {
        None
    }
}

fn mentions(text: &str, entities: &[Entity], account: &User) -> Vec<UserKey> {
    let mut users = Vec::new();
    for entity in entities {
        let Some(label) = entity_text(text, entity) else {
            continue;
        };
        let id = match entity.kind.as_str() {
            "text_mention" => entity.user.as_ref().map(|user| user.id),
            "mention" => account.username.as_deref().and_then(|username| {
                label
                    .strip_prefix('@')
                    .filter(|name| name.eq_ignore_ascii_case(username))
                    .map(|_| account.id)
            }),
            _ => None,
        };
        if let Some(id) = id {
            let key = user_key(id);
            if !users.contains(&key) {
                users.push(key);
            }
        }
    }
    users
}

pub(crate) fn decode_update(
    update: &Update,
    connection: &ConnectionId,
    account: &User,
    images: bool,
) -> Vec<ChatEvent> {
    // Edits are not new conversation observations, including join service edits.
    if update.edited_message.is_some() {
        return Vec::new();
    }
    let Some(message) = update.message.as_ref() else {
        return Vec::new();
    };
    let room = room_ref(message);
    let mut events = Vec::new();
    // Telegram can attribute a join service message to a bot. Decode its human
    // members independently of the ordinary-message author exclusions.
    for member in &message.new_chat_members {
        if !member.is_bot && member.id != account.id {
            events.push(ChatEvent::MemberJoined {
                room: room.clone(),
                user: user_ref(member),
            });
        }
    }
    let Some(author) = message.from.as_ref() else {
        return events;
    };
    if author.is_bot || author.id == account.id || message.sender_chat.is_some() {
        return events;
    }
    let (text, entities) = match message.text.as_deref() {
        Some(text) => (text, message.entities.as_slice()),
        None => (
            message.caption.as_deref().unwrap_or_default(),
            message.caption_entities.as_slice(),
        ),
    };
    let photo = if images {
        message.photo.iter().max_by_key(|photo| {
            (
                u64::from(photo.width) * u64::from(photo.height),
                photo.file_size.unwrap_or_default(),
            )
        })
    } else {
        None
    };
    if text.trim().is_empty() && photo.is_none() {
        return events;
    }
    events.push(ChatEvent::MessageCreated(IncomingMessage {
        id: message_key(message.chat.id, message.message_id),
        room,
        author: user_ref(author),
        sent_at_unix_ms: message.date.saturating_mul(1000),
        text: text.to_owned(),
        mentions: mentions(text, entities, account),
        reply_to: message.reply_to_message.as_ref().map(|reply| MessageRef {
            id: message_key(reply.chat.id, reply.message_id),
            room: room_key(reply.chat.id),
            author: if reply.sender_chat.is_none() {
                reply.from.as_ref().map(|user| user_key(user.id))
            } else {
                None
            },
        }),
        attachments: photo
            .into_iter()
            .map(|photo| AttachmentRef {
                kind: MediaKind::Image,
                mime: Some("image/jpeg".to_owned()),
                size: photo.file_size,
                source: MediaSource::Native {
                    connection: connection.clone(),
                    id: photo.file_id.clone(),
                },
            })
            .collect(),
    }));
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn account() -> User {
        serde_json::from_value(json!({
            "id": 99, "is_bot": true, "first_name": "Bot", "username": "MyBot",
            "can_read_all_group_messages": true
        }))
        .unwrap()
    }

    fn human(id: i64) -> Value {
        json!({"id": id, "is_bot": false, "first_name": "Ayşe", "last_name": "Yılmaz"})
    }

    fn native_message() -> Value {
        json!({
            "message_id": 17,
            "date": 1_700_000_000,
            "chat": {"id": -100123, "type": "supergroup", "title": "Sohbet"},
            "from": human(7),
            "text": "Merhaba"
        })
    }

    fn update(message: Value) -> Update {
        serde_json::from_value(json!({"update_id": 1200, "message": message})).unwrap()
    }

    fn decode(update: &Update, images: bool) -> Vec<ChatEvent> {
        decode_update(
            update,
            &ConnectionId::new("telegram-test"),
            &account(),
            images,
        )
    }

    fn incoming(events: &[ChatEvent]) -> &IncomingMessage {
        assert_eq!(events.len(), 1);
        let ChatEvent::MessageCreated(message) = &events[0] else {
            panic!("expected an ordinary message")
        };
        message
    }

    #[test]
    fn text_reply_and_native_identity_are_preserved() {
        let mut native = native_message();
        let mut reply = native_message();
        reply["message_id"] = json!(16);
        reply["from"] = json!({
            "id": 99, "is_bot": true, "first_name": "Bot"
        });
        native["reply_to_message"] = reply;
        let update = update(native);
        assert_eq!(update.update_id, 1200);
        let events = decode(&update, false);
        let message = incoming(&events);
        assert_eq!(message.id.as_str(), "telegram:message:-100123:17");
        assert_eq!(message.room.id.as_str(), "telegram:room:-100123");
        assert_eq!(message.room.kind, RoomKind::Group);
        assert_eq!(message.room.name.as_deref(), Some("Sohbet"));
        assert!(message.room.space.is_none());
        assert_eq!(message.author.id, user_key(7));
        assert_eq!(message.author.display_name, "Ayşe Yılmaz");
        assert_eq!(message.author.automation, Automation::Human);
        assert_eq!(message.author.stability, IdentityStability::Persistent);
        assert_eq!(message.text, "Merhaba");
        assert_eq!(message.sent_at_unix_ms, 1_700_000_000_000);
        let reply = message.reply_to.as_ref().unwrap();
        assert_eq!(reply.id, message_key(-100123, 16));
        assert_eq!(reply.room, room_key(-100123));
        assert_eq!(reply.author, Some(user_key(99)));
        assert!(message.attachments.is_empty());
    }

    #[test]
    fn mentions_use_utf16_entities_and_deduplicate_users() {
        let mut native = native_message();
        native["text"] = json!("😀 @mYbOt Ayşe @Other MyBot");
        native["entities"] = json!([
            {"type": "mention", "offset": 3, "length": 6},
            {"type": "text_mention", "offset": 10, "length": 4, "user": human(8)},
            {"type": "mention", "offset": 15, "length": 6},
            {"type": "text_mention", "offset": 10, "length": 4, "user": human(8)},
            {"type": "bold", "offset": 22, "length": 5}
        ]);
        let events = decode(&update(native), false);
        assert_eq!(incoming(&events).mentions, vec![user_key(99), user_key(8)]);
    }

    #[test]
    fn malformed_utf16_ranges_cannot_create_mentions() {
        let mut native = native_message();
        native["text"] = json!("😀 @MyBot");
        native["entities"] = json!([
            {"type": "text_mention", "offset": 1, "length": 1, "user": human(8)},
            {"type": "text_mention", "offset": 0, "length": 1, "user": human(9)},
            {"type": "mention", "offset": 3, "length": 600},
            {"type": "mention", "offset": 4, "length": 5}
        ]);
        let events = decode(&update(native), false);
        assert!(incoming(&events).mentions.is_empty());
    }

    #[test]
    fn unstructured_username_text_does_not_mention_bot() {
        let mut native = native_message();
        native["text"] = json!("@MyBot is literal without a native entity");
        assert!(incoming(&decode(&update(native), false))
            .mentions
            .is_empty());
    }

    #[test]
    fn photo_caption_topic_and_connection_scoped_native_media() {
        let mut native = native_message();
        native.as_object_mut().unwrap().remove("text");
        native["caption"] = json!("😀 @MyBot");
        native["caption_entities"] = json!([
            {"type": "mention", "offset": 3, "length": 6}
        ]);
        native["message_thread_id"] = json!(42);
        native["photo"] = json!([
            {"file_id": "small", "width": 90, "height": 90, "file_size": 900},
            {"file_id": "original-file-id", "width": 1280, "height": 720, "file_size": 9000},
            {"file_id": "medium", "width": 320, "height": 180}
        ]);
        let update = update(native);
        assert!(command_text(update.message.as_ref().unwrap()).is_none());
        let events = decode(&update, true);
        let message = incoming(&events);
        assert_eq!(message.room.thread, Some(thread_key(-100123, 42)));
        assert_eq!(message.text, "😀 @MyBot");
        assert_eq!(message.mentions, vec![user_key(99)]);
        assert_eq!(message.attachments.len(), 1);
        let attachment = &message.attachments[0];
        assert_eq!(attachment.kind, MediaKind::Image);
        assert_eq!(attachment.mime.as_deref(), Some("image/jpeg"));
        assert_eq!(attachment.size, Some(9000));
        let MediaSource::Native { connection, id } = &attachment.source else {
            panic!("Telegram media must remain native until transport resolution")
        };
        assert_eq!(connection.as_str(), "telegram-test");
        assert_eq!(id, "original-file-id");
        let without_images = decode(&update, false);
        assert!(incoming(&without_images).attachments.is_empty());
    }

    #[test]
    fn image_only_messages_require_enabled_images() {
        let mut native = native_message();
        native["text"] = json!(" \n ");
        native["photo"] = json!([
            {"file_id": "photo", "width": 100, "height": 100}
        ]);
        let photo_update = update(native);
        assert!(decode(&photo_update, false).is_empty());
        assert_eq!(incoming(&decode(&photo_update, true)).attachments.len(), 1);
        assert!(decode(&update(native_message_without_content()), true).is_empty());
    }

    fn native_message_without_content() -> Value {
        let mut native = native_message();
        native.as_object_mut().unwrap().remove("text");
        native
    }

    #[test]
    fn bot_self_anonymous_and_sender_chat_messages_are_filtered() {
        let mut bot_message = native_message();
        bot_message["from"]["is_bot"] = json!(true);
        assert!(decode(&update(bot_message), true).is_empty());
        let mut self_message = native_message();
        self_message["from"]["id"] = json!(99);
        assert!(decode(&update(self_message), true).is_empty());
        let mut anonymous = native_message();
        anonymous.as_object_mut().unwrap().remove("from");
        assert!(decode(&update(anonymous), true).is_empty());
        let mut sender_chat = native_message();
        sender_chat["sender_chat"] = json!({"id": -100555, "type": "channel"});
        assert!(decode(&update(sender_chat), true).is_empty());
    }

    #[test]
    fn bot_authored_joins_keep_only_humans_and_forum_scope() {
        let mut native = native_message_without_content();
        native["from"] = json!({"id": 99, "is_bot": true, "first_name": "Bot"});
        native["message_thread_id"] = json!(42);
        native["new_chat_members"] = json!([
            human(7),
            {"id": 8, "is_bot": true, "first_name": "Other bot"},
            human(99),
            human(9)
        ]);
        let events = decode(&update(native), false);
        assert_eq!(events.len(), 2);
        for (event, expected) in events.iter().zip([7, 9]) {
            let ChatEvent::MemberJoined { room, user } = event else {
                panic!("expected only human join events")
            };
            assert_eq!(room.id, room_key(-100123));
            assert_eq!(room.thread, Some(thread_key(-100123, 42)));
            assert_eq!(user.id, user_key(expected));
        }
    }

    #[test]
    fn edited_and_unhandled_updates_are_not_observations() {
        let edited: Update = serde_json::from_value(json!({
            "update_id": 1201, "edited_message": native_message()
        }))
        .unwrap();
        assert!(edited.message.is_none());
        assert!(edited.edited_message.is_some());
        assert!(decode(&edited, true).is_empty());
        let unknown: Update = serde_json::from_value(json!({
            "update_id": 1202, "channel_post": native_message()
        }))
        .unwrap();
        assert!(decode(&unknown, true).is_empty());
    }

    #[test]
    fn private_rooms_commands_and_bot_metadata_are_native() {
        let mut native = native_message();
        native["chat"] = json!({
            "id": 7, "type": "private", "first_name": "Ayşe", "last_name": "Yılmaz"
        });
        native["text"] = json!("/help@MyBot");
        let update = update(native);
        assert_eq!(
            command_text(update.message.as_ref().unwrap()),
            Some("/help@MyBot")
        );
        let events = decode(&update, false);
        assert_eq!(incoming(&events).room.kind, RoomKind::Direct);
        assert_eq!(incoming(&events).room.name.as_deref(), Some("Ayşe Yılmaz"));
        let account = account();
        assert_eq!(account.can_read_all_group_messages, Some(true));
        assert_eq!(user_ref(&account).automation, Automation::Bot);
        assert_eq!(user_ref(&account).username.as_deref(), Some("MyBot"));
    }

    #[test]
    fn negative_chat_keys_round_trip_and_keep_message_topic_scope() {
        let room = room_key(-100123);
        let message = message_key(-100123, 17);
        let topic = thread_key(-100123, 42);
        assert_eq!(room.as_str().parse::<RoomKey>().unwrap(), room);
        assert_eq!(message.as_str().parse::<MessageKey>().unwrap(), message);
        assert_eq!(topic.as_str().parse::<ThreadKey>().unwrap(), topic);
        assert_eq!(
            message.components().collect::<Result<Vec<_>, _>>().unwrap(),
            vec!["-100123", "17"]
        );
        assert_eq!(topic.as_str(), "telegram:thread:-100123:42");
        assert_ne!(message, message_key(-100456, 17));
        assert_ne!(topic, thread_key(-100456, 42));
    }
}

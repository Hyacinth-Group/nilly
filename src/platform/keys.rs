//! Canonical identity-to-storage mapping. Legacy numeric names are handled only
//! by the offline schema migration, never by runtime lookup aliases.
use super::types::{encode_component, ConversationKey, RoomKey, SpaceKey, UserKey};

pub fn person_record(id: &UserKey) -> String {
    format!("kisiler/{id}.md")
}
pub fn room_record(id: &ConversationKey) -> String {
    match &id.thread {
        Some(thread) => format!("kanallar/{}/threads/{thread}.md", id.room),
        None => format!("kanallar/{}.md", id.room),
    }
}
pub fn observation_record(id: &ConversationKey) -> String {
    room_record(id).replacen("kanallar/", "kanal-kimlikleri/", 1)
}
pub fn parse_person_record(key: &str) -> Option<UserKey> {
    key.strip_prefix("kisiler/")?
        .strip_suffix(".md")?
        .parse()
        .ok()
}
pub fn parse_room_record(key: &str) -> Option<ConversationKey> {
    let value = key.strip_prefix("kanallar/")?.strip_suffix(".md")?;
    let (room, thread): (RoomKey, Option<super::types::ThreadKey>) =
        match value.split_once("/threads/") {
            Some((room, thread)) => (room.parse().ok()?, Some(thread.parse().ok()?)),
            None => (value.parse().ok()?, None),
        };
    if thread
        .as_ref()
        .is_some_and(|thread: &super::types::ThreadKey| thread.platform() != room.platform())
    {
        return None;
    }
    Some(ConversationKey { room, thread })
}
pub fn parse_space_line(line: &str) -> Option<SpaceKey> {
    line.trim().parse().ok()
}

/// Only archive filenames are real filesystem paths. Escape reserved characters
/// in each NEW path segment, without renaming historical archive artifacts.
pub fn archive_relative(rel: &str) -> String {
    let mut result = String::with_capacity(rel.len());
    for (index, component) in rel.split('/').enumerate() {
        if index != 0 {
            result.push('/');
        }
        result.push_str(&encode_component(component));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::types::PlatformKind;
    #[test]
    fn storage_keys_round_trip_without_platform_collisions() {
        let discord = UserKey::discord(42);
        let telegram = UserKey::new(PlatformKind::Telegram, &["42"]).unwrap();
        assert_eq!(person_record(&discord), "kisiler/discord:user:42.md");
        assert_ne!(person_record(&discord), person_record(&telegram));
        assert_eq!(
            parse_person_record(&person_record(&telegram)),
            Some(telegram)
        );
        let matrix = RoomKey::new(PlatformKind::Matrix, &["!room:example.org"]).unwrap();
        let conversation = ConversationKey {
            room: matrix,
            thread: None,
        };
        assert_eq!(
            parse_room_record(&room_record(&conversation)),
            Some(conversation)
        );
        assert_eq!(
            parse_space_line(" discord:space:9 "),
            Some(SpaceKey::discord(9))
        );
        assert!(parse_person_record("kisiler/42.md").is_none());
        assert!(parse_room_record("kanallar/7.md").is_none());
        assert!(parse_space_line("9").is_none());
    }
    #[test]
    fn threaded_records_extend_v2_keys_without_flattening() {
        let plain = ConversationKey {
            room: RoomKey::discord(7),
            thread: None,
        };
        assert_eq!(room_record(&plain), "kanallar/discord:room:7.md");
        let first = ConversationKey {
            room: plain.room.clone(),
            thread: Some(super::super::types::ThreadKey::discord(81)),
        };
        let second = ConversationKey {
            room: plain.room.clone(),
            thread: Some(super::super::types::ThreadKey::discord(82)),
        };
        assert_eq!(
            room_record(&first),
            "kanallar/discord:room:7/threads/discord:thread:81.md"
        );
        assert_eq!(parse_room_record(&room_record(&first)), Some(first.clone()));
        assert_ne!(room_record(&first), room_record(&second));
        assert!(
            parse_room_record("kanallar/discord:room:7/threads/matrix:thread:%24event.md")
                .is_none()
        );
        assert!(
            parse_room_record("kanallar/discord:room:7/threads/discord:thread:81/extra.md")
                .is_none()
        );
    }
    #[test]
    fn new_archive_names_are_safe_and_historical_names_remain_stable() {
        assert_eq!(archive_relative("kisiler/42.md"), "kisiler/42.md");
        assert_eq!(
            archive_relative("kisiler/discord:user:42.md"),
            "kisiler/discord%3Auser%3A42.md"
        );
        assert_eq!(
            archive_relative("kisiler/matrix:user:%40alice%3Aexample.org.md"),
            "kisiler/matrix%3Auser%3A%2540alice%253Aexample.org.md"
        );
    }
}

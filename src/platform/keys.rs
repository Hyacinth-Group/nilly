//! Canonical identity-to-storage mapping. Legacy numeric names are handled only
//! by the offline schema migration, never by runtime lookup aliases.
use super::types::{encode_component, RoomKey, SpaceKey, UserKey};

pub fn person_record(id: &UserKey) -> String {
    format!("kisiler/{id}.md")
}
pub fn room_record(id: &RoomKey) -> String {
    format!("kanallar/{id}.md")
}
pub fn parse_person_record(key: &str) -> Option<UserKey> {
    key.strip_prefix("kisiler/")?
        .strip_suffix(".md")?
        .parse()
        .ok()
}
pub fn parse_room_record(key: &str) -> Option<RoomKey> {
    key.strip_prefix("kanallar/")?
        .strip_suffix(".md")?
        .parse()
        .ok()
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
        assert_eq!(parse_room_record(&room_record(&matrix)), Some(matrix));
        assert_eq!(
            parse_space_line(" discord:space:9 "),
            Some(SpaceKey::discord(9))
        );
        assert!(parse_person_record("kisiler/42.md").is_none());
        assert!(parse_room_record("kanallar/7.md").is_none());
        assert!(parse_space_line("9").is_none());
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

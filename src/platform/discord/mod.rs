//! Discord-native decoding, rendering, and private outbound SDK operations.
pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod render;
mod transport;
pub use transport::DiscordAdapter;

use super::types::{KeyError, MessageKey, RoomKey, SpaceKey, UserKey};
use serenity::all::{ChannelId, GuildId, MessageId, UserId};

fn native_id(value: &str, prefix: &str) -> Result<u64, KeyError> {
    let component = value.strip_prefix(prefix).ok_or(KeyError)?;
    if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) {
        return Err(KeyError);
    }
    let id = component.parse::<u64>().map_err(|_| KeyError)?;
    if id == 0 {
        return Err(KeyError);
    }
    Ok(id)
}

pub(crate) fn native_room(key: &RoomKey) -> Result<ChannelId, KeyError> {
    native_id(key.as_str(), "discord:room:").map(ChannelId::new)
}
pub(crate) fn native_message(key: &MessageKey) -> Result<MessageId, KeyError> {
    native_id(key.as_str(), "discord:message:").map(MessageId::new)
}
pub(crate) fn native_space(key: &SpaceKey) -> Result<GuildId, KeyError> {
    native_id(key.as_str(), "discord:space:").map(GuildId::new)
}
pub(crate) fn native_user(key: &UserKey) -> Result<UserId, KeyError> {
    native_id(key.as_str(), "discord:user:").map(UserId::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discord_native_ids_only_accept_nonzero_discord_u64_components() {
        assert_eq!(
            native_room(&RoomKey::discord(7)).unwrap(),
            ChannelId::new(7)
        );
        assert_eq!(
            native_message(&MessageKey::discord(81)).unwrap(),
            MessageId::new(81)
        );
        assert_eq!(
            native_space(&SpaceKey::discord(9)).unwrap(),
            GuildId::new(9)
        );
        assert_eq!(native_user(&UserKey::discord(42)).unwrap(), UserId::new(42));
        for input in [
            "telegram:user:42",
            "discord:user:0",
            "discord:user:18446744073709551616",
            "discord:user:account",
            "discord:user:%2B42",
        ] {
            assert!(native_user(&input.parse().unwrap()).is_err(), "{input}");
        }
        assert!(native_room(&RoomKey::terminal()).is_err());
        assert!(native_message(&"matrix:message:%24event".parse().unwrap()).is_err());
        assert!(native_space(&"telegram:space:9".parse().unwrap()).is_err());
    }
}

//! Transport-independent identities and ingress payloads. Native components are
//! percent-encoded once; credentials and connection names never enter durable IDs.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, fmt, str::FromStr, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformKind {
    Discord,
    Telegram,
    Slack,
    Matrix,
    Terminal,
}
impl PlatformKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Matrix => "matrix",
            Self::Terminal => "terminal",
        }
    }
}
impl FromStr for PlatformKind {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "discord" => Ok(Self::Discord),
            "telegram" => Ok(Self::Telegram),
            "slack" => Ok(Self::Slack),
            "matrix" => Ok(Self::Matrix),
            "terminal" => Ok(Self::Terminal),
            _ => Err(KeyError),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyError;
impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid canonical platform identity")
    }
}
impl std::error::Error for KeyError {}

/// Encode an opaque UTF-8 component, preserving only filesystem-safe unreserved bytes.
pub fn encode_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}
/// Decode a canonical component; reject noncanonical escapes and unsafe raw bytes.
pub fn decode_component(value: &str) -> Result<String, KeyError> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(b) = input.next() {
        if b == b'%' {
            let a = input.next().ok_or(KeyError)?;
            let c = input.next().ok_or(KeyError)?;
            let digit = |v: u8| match v {
                b'0'..=b'9' => Ok(v - b'0'),
                b'A'..=b'F' => Ok(v - b'A' + 10),
                _ => Err(KeyError),
            };
            bytes.push((digit(a)? << 4) | digit(c)?);
        } else if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            bytes.push(b);
        } else {
            return Err(KeyError);
        }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| KeyError)?;
    if encode_component(&decoded) != value {
        return Err(KeyError);
    }
    Ok(decoded)
}
fn key(platform: PlatformKind, entity: &str, components: &[&str]) -> Arc<str> {
    let mut value = format!("{}:{entity}", platform.as_str());
    for component in components {
        value.push(':');
        value.push_str(&encode_component(component));
    }
    value.into()
}
fn validate(value: &str, entity: &str) -> Result<(), KeyError> {
    let mut pieces = value.split(':');
    let platform: PlatformKind = pieces.next().ok_or(KeyError)?.parse()?;
    if pieces.next() != Some(entity) {
        return Err(KeyError);
    }
    let expected = match (platform, entity) {
        (PlatformKind::Slack, "message") => 3,
        (PlatformKind::Slack, _) | (PlatformKind::Telegram, "message") => 2,
        (PlatformKind::Terminal, _) => 2,
        _ => 1,
    };
    let mut count = 0;
    for component in pieces {
        if component.is_empty() || decode_component(component).is_err() {
            return Err(KeyError);
        }
        count += 1;
    }
    if count != expected {
        return Err(KeyError);
    }
    Ok(())
}
macro_rules! identity {
    ($name:ident, $entity:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Arc<str>);
        impl $name {
            pub fn new(platform: PlatformKind, components: &[&str]) -> Result<Self, KeyError> {
                let value = key(platform, $entity, components);
                validate(&value, $entity)?;
                Ok(Self(value))
            }
            pub fn discord(id: u64) -> Self {
                Self(key(PlatformKind::Discord, $entity, &[&id.to_string()]))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn platform(&self) -> PlatformKind {
                self.0
                    .split(':')
                    .next()
                    .expect("validated key")
                    .parse()
                    .expect("validated platform")
            }
            pub fn components(&self) -> impl Iterator<Item = Result<String, KeyError>> + '_ {
                self.0.split(':').skip(2).map(decode_component)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl FromStr for $name {
            type Err = KeyError;
            fn from_str(s: &str) -> Result<Self, KeyError> {
                validate(s, $entity)?;
                Ok(Self(s.into()))
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}
identity!(UserKey, "user");
identity!(SpaceKey, "space");
identity!(RoomKey, "room");
identity!(MessageKey, "message");
identity!(ThreadKey, "thread");
identity!(EventKey, "event");
impl Default for UserKey {
    fn default() -> Self {
        Self(key(PlatformKind::Terminal, "user", &["local", "anonymous"]))
    }
}
impl RoomKey {
    pub fn terminal() -> Self {
        Self(key(PlatformKind::Terminal, "room", &["local", "bench"]))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(Arc<str>);
impl ConnectionId {
    pub fn new(value: &str) -> Self {
        Self(Arc::from(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::borrow::Borrow<str> for ConnectionId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Serialize for ConnectionId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for ConnectionId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(&String::deserialize(d)?))
    }
}

/// Chat bookkeeping includes native topic/thread scope, when a transport exposes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationKey {
    pub room: RoomKey,
    pub thread: Option<ThreadKey>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Automation {
    Human,
    Bot,
    Integration,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityStability {
    Persistent,
    SessionOnly,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    pub id: UserKey,
    pub display_name: String,
    pub username: Option<String>,
    pub automation: Automation,
    pub stability: IdentityStability,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomKind {
    Group,
    Channel,
    Direct,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomRef {
    pub id: RoomKey,
    pub space: Option<SpaceKey>,
    pub kind: RoomKind,
    pub name: Option<String>,
    pub thread: Option<ThreadKey>,
}
impl RoomRef {
    pub fn conversation(&self) -> ConversationKey {
        ConversationKey {
            room: self.id.clone(),
            thread: self.thread.clone(),
        }
    }
    pub fn from_key(id: RoomKey) -> Self {
        Self {
            id,
            space: None,
            kind: RoomKind::Group,
            name: None,
            thread: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRef {
    pub id: MessageKey,
    pub room: RoomKey,
    pub author: Option<UserKey>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    File,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaSource {
    PublicUrl(String),
    Native {
        connection: ConnectionId,
        id: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub kind: MediaKind,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub source: MediaSource,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    pub id: MessageKey,
    pub room: RoomRef,
    pub author: UserRef,
    pub sent_at_unix_ms: i64,
    pub text: String,
    pub mentions: Vec<UserKey>,
    pub reply_to: Option<MessageRef>,
    pub attachments: Vec<AttachmentRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Emoji {
    Unicode(String),
    Custom {
        platform: PlatformKind,
        id: String,
        label: Option<String>,
        animated: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionEvent {
    pub room: RoomRef,
    pub actor: UserRef,
    pub target: MessageRef,
    pub emoji: Emoji,
    pub original_text: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInvocation {
    pub name: String,
    pub arguments: BTreeMap<String, ActionValue>,
    pub actor: UserRef,
    pub room: RoomRef,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatEvent {
    RoomReady(RoomRef),
    MessageCreated(IncomingMessage),
    ReactionAdded(ReactionEvent),
    MemberJoined {
        room: RoomRef,
        user: UserRef,
    },
    CommandInvoked(CommandInvocation),
    CapabilitiesChanged {
        room: RoomKey,
        capabilities: super::capabilities::Capabilities,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub connection: ConnectionId,
    pub event_id: EventKey,
    pub occurred_at_unix_ms: i64,
    pub event: ChatEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_components_round_trip() {
        for input in [
            "-100123",
            "@alice:example.org",
            "!room:example.org",
            "$event",
            "1700000000.000123",
            ":/%\\",
            "Türkçe",
            "..",
        ] {
            assert_eq!(decode_component(&encode_component(input)).unwrap(), input);
        }
        assert_eq!(UserKey::discord(42).as_str(), "discord:user:42");
        assert_eq!(
            RoomKey::new(PlatformKind::Telegram, &["-100123"])
                .unwrap()
                .as_str(),
            "telegram:room:-100123"
        );
        let message =
            MessageKey::new(PlatformKind::Slack, &["T123", "C789", "1700000000.000123"]).unwrap();
        assert_eq!(message.to_string().parse::<MessageKey>().unwrap(), message);
        assert_ne!(
            UserKey::discord(42),
            UserKey::new(PlatformKind::Telegram, &["42"]).unwrap()
        );
    }
    #[test]
    fn key_parser_rejects_unsafe_noncanonical_and_wrong_entities() {
        for key in [
            "matrix:user:@alice:example.org",
            "matrix:user:%40alice%3aexample.org",
            "discord:room:42",
            "unknown:user:42",
            "discord:user:a/b",
            "discord:user:%41",
            "discord:user:",
            "slack:user:U456",
        ] {
            assert!(key.parse::<UserKey>().is_err(), "{key}");
        }
        assert_eq!(
            UserKey::new(PlatformKind::Matrix, &["@alice:example.org"])
                .unwrap()
                .as_str(),
            "matrix:user:%40alice%3Aexample.org"
        );
    }
}

//! Borrowed outgoing payloads avoid copying stream text before native serialization.
//! Native markup, SDK objects and interaction tokens do not cross this boundary.
use super::{capabilities::Capabilities, types::*};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, path::Path};

#[derive(Debug, Clone, Copy, Default)]
pub struct MentionPolicy<'a> {
    pub users: &'a [UserKey],
    pub reply_user: bool,
    /// A deliberate welcome mention. The adapter supplies native syntax.
    pub prefix_user: Option<&'a UserKey>,
}
#[derive(Debug, Clone, Copy)]
pub enum OutgoingAttachment<'a> {
    LocalPath(&'a Path),
    Bytes {
        filename: &'a str,
        data: &'a [u8],
        mime: Option<&'a str>,
    },
}
#[derive(Debug, Clone, Copy)]
pub enum PresentationPayload<'a> {
    Card {
        title: &'a str,
        description: &'a str,
    },
    /// Append the private thought-detail affordance without rewriting content.
    ThoughtReveal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThoughtPresentation {
    Show,
    Hide,
    Silent,
    Off,
}
#[derive(Debug, Clone)]
pub struct OutgoingMessage<'a> {
    pub text: Cow<'a, str>,
    pub reply_to: Option<&'a MessageRef>,
    pub mentions: MentionPolicy<'a>,
    pub attachments: &'a [OutgoingAttachment<'a>],
    pub presentation: Option<PresentationPayload<'a>>,
}
impl<'a> OutgoingMessage<'a> {
    pub fn text(text: &'a str) -> Self {
        Self {
            text: Cow::Borrowed(text),
            reply_to: None,
            mentions: MentionPolicy::default(),
            attachments: &[],
            presentation: None,
        }
    }
}
/// The caller advances accepted text/version only after a successful edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageReceipt {
    pub connection: ConnectionId,
    pub message: MessageRef,
    pub room: RoomRef,
    pub last_accepted_text: String,
    pub version: u64,
}
#[derive(Debug, Clone, Copy)]
pub enum NickTarget<'a> {
    Room(&'a RoomRef),
    Space(&'a SpaceKey),
    Account,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequest {
    pub cursor: Option<String>,
    pub cutoff_unix_ms: Option<i64>,
    pub limit: usize,
}
impl Default for HistoryRequest {
    fn default() -> Self {
        Self {
            cursor: None,
            cutoff_unix_ms: None,
            limit: 100,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryOrder {
    NewestFirst,
    OldestFirst,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryLimitation {
    Restricted,
    Undecryptable,
    Bounded,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPage {
    pub messages: Vec<IncomingMessage>,
    pub next_cursor: Option<String>,
    pub limitations: Vec<HistoryLimitation>,
    pub order: HistoryOrder,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelMedia {
    PublicUrl(String),
    DataUrl(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Connecting,
    Ready,
    Disconnected,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub room: RoomRef,
    pub capabilities: Capabilities,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub connection: ConnectionId,
    pub platform: PlatformKind,
    pub status: ConnectionStatus,
    pub account: Option<UserRef>,
    pub rooms: Vec<RoomSnapshot>,
}

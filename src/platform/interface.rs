//! Native return-position futures keep send/edit/reaction hot paths allocation-free.
use super::{capabilities::Capabilities, discord::DiscordAdapter, outgoing::*, types::*};
use std::{fmt, future::Future, time::Duration};

pub type PlatformResult<T> = Result<T, PlatformError>;
#[derive(Debug)]
pub enum PlatformError {
    Unsupported(&'static str),
    PermissionDenied,
    NotFound,
    RateLimited { retry_after: Option<Duration> },
    Disconnected,
    Transport(String),
    InvalidIdentity(KeyError),
}
impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(operation) => {
                write!(f, "unsupported platform operation: {operation}")
            }
            Self::PermissionDenied => f.write_str("platform permission denied"),
            Self::NotFound => f.write_str("platform resource not found"),
            Self::RateLimited { retry_after } => {
                write!(f, "platform rate limited (retry after {retry_after:?})")
            }
            Self::Disconnected => f.write_str("platform disconnected"),
            Self::Transport(error) => f.write_str(error),
            Self::InvalidIdentity(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PlatformError {}
impl From<KeyError> for PlatformError {
    fn from(error: KeyError) -> Self {
        Self::InvalidIdentity(error)
    }
}

/// Implementations own native sessions/caches and enforce room permissions. Unknown
/// or unsupported operations must return errors rather than succeed silently.
pub trait Platform: Send + Sync {
    fn connection_id(&self) -> &ConnectionId;
    fn self_user(&self) -> &UserRef;
    fn capabilities(&self, room: &RoomRef) -> Capabilities;
    fn render_stream(
        &self,
        room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String>;
    fn render_final(
        &self,
        room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String>;
    fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> impl Future<Output = PlatformResult<MessageReceipt>> + Send + 'a;
    fn edit<'a>(
        &'a self,
        sent: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> impl Future<Output = PlatformResult<()>> + Send + 'a;
    fn delete<'a>(
        &'a self,
        sent: &'a MessageReceipt,
    ) -> impl Future<Output = PlatformResult<()>> + Send + 'a;
    fn react<'a>(
        &'a self,
        target: &'a MessageRef,
        emoji: &'a Emoji,
    ) -> impl Future<Output = PlatformResult<()>> + Send + 'a;
    fn typing<'a>(
        &'a self,
        room: &'a RoomRef,
    ) -> impl Future<Output = PlatformResult<()>> + Send + 'a;
    fn set_nickname<'a>(
        &'a self,
        target: NickTarget<'a>,
        name: &'a str,
    ) -> impl Future<Output = PlatformResult<()>> + Send + 'a;
    fn fetch_history<'a>(
        &'a self,
        room: &'a RoomRef,
        page: HistoryRequest,
    ) -> impl Future<Output = PlatformResult<HistoryPage>> + Send + 'a;
    fn list_rooms(&self) -> impl Future<Output = PlatformResult<Vec<RoomRef>>> + Send;
    fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> impl Future<Output = PlatformResult<ModelMedia>> + Send + 'a;
}

/// Add a concrete SDK variant when that adapter is implemented. Match delegation
/// generates one statically sized future; no boxed trait objects or plugin registry.
pub enum Adapter {
    Discord(DiscordAdapter),
}
impl Platform for Adapter {
    fn connection_id(&self) -> &ConnectionId {
        match self {
            Self::Discord(adapter) => adapter.connection_id(),
        }
    }
    fn self_user(&self) -> &UserRef {
        match self {
            Self::Discord(adapter) => adapter.self_user(),
        }
    }
    fn capabilities(&self, room: &RoomRef) -> Capabilities {
        match self {
            Self::Discord(adapter) => adapter.capabilities(room),
        }
    }
    fn render_stream(
        &self,
        room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        text: &str,
        done: bool,
    ) -> Vec<String> {
        match self {
            Self::Discord(adapter) => adapter.render_stream(room, mode, thought, text, done),
        }
    }
    fn render_final(
        &self,
        room: &RoomRef,
        mode: ThoughtPresentation,
        thought: &str,
        lines: &[String],
    ) -> Vec<String> {
        match self {
            Self::Discord(adapter) => adapter.render_final(room, mode, thought, lines),
        }
    }
    async fn send<'a>(
        &'a self,
        room: &'a RoomRef,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<MessageReceipt> {
        match self {
            Self::Discord(adapter) => adapter.send(room, body).await,
        }
    }
    async fn edit<'a>(
        &'a self,
        sent: &'a MessageReceipt,
        body: &'a OutgoingMessage<'a>,
    ) -> PlatformResult<()> {
        match self {
            Self::Discord(adapter) => adapter.edit(sent, body).await,
        }
    }
    async fn delete<'a>(&'a self, sent: &'a MessageReceipt) -> PlatformResult<()> {
        match self {
            Self::Discord(adapter) => adapter.delete(sent).await,
        }
    }
    async fn react<'a>(&'a self, target: &'a MessageRef, emoji: &'a Emoji) -> PlatformResult<()> {
        match self {
            Self::Discord(adapter) => adapter.react(target, emoji).await,
        }
    }
    async fn typing<'a>(&'a self, room: &'a RoomRef) -> PlatformResult<()> {
        match self {
            Self::Discord(adapter) => adapter.typing(room).await,
        }
    }
    async fn set_nickname<'a>(
        &'a self,
        target: NickTarget<'a>,
        name: &'a str,
    ) -> PlatformResult<()> {
        match self {
            Self::Discord(adapter) => adapter.set_nickname(target, name).await,
        }
    }
    async fn fetch_history<'a>(
        &'a self,
        room: &'a RoomRef,
        page: HistoryRequest,
    ) -> PlatformResult<HistoryPage> {
        match self {
            Self::Discord(adapter) => adapter.fetch_history(room, page).await,
        }
    }
    async fn list_rooms(&self) -> PlatformResult<Vec<RoomRef>> {
        match self {
            Self::Discord(adapter) => adapter.list_rooms().await,
        }
    }
    async fn resolve_media<'a>(
        &'a self,
        attachment: &'a AttachmentRef,
    ) -> PlatformResult<ModelMedia> {
        match self {
            Self::Discord(adapter) => adapter.resolve_media(attachment).await,
        }
    }
}

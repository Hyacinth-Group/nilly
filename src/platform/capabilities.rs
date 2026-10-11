//! Capabilities are permission-sensitive room facts, not protocol marketing claims.
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageVisibility {
    Ordinary,
    MentionsOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryAccess {
    None,
    Bounded,
    Paginated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NicknameScope {
    None,
    Room,
    Space,
    Account,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountUnit {
    UnicodeScalars,
    Utf16Units,
    EncodedBytes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLimits {
    pub text: usize,
    pub count_unit: CountUnit,
}
impl OutputLimits {
    pub const DISCORD: Self = Self {
        text: 1900,
        count_unit: CountUnit::UnicodeScalars,
    };
    pub fn count(self, text: &str) -> usize {
        match self.count_unit {
            CountUnit::UnicodeScalars => text.chars().count(),
            CountUnit::Utf16Units => text.encode_utf16().count(),
            CountUnit::EncodedBytes => text.len(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub visibility: MessageVisibility,
    pub history: HistoryAccess,
    pub decryption_available: bool,
    pub replies: bool,
    pub threads: bool,
    pub send: bool,
    pub react: bool,
    pub observe_reactions: bool,
    pub edit: bool,
    pub delete: bool,
    pub typing: bool,
    pub nickname: NicknameScope,
    pub rich_cards: bool,
    pub components: bool,
    pub modals: bool,
    pub native_commands: bool,
    pub private_results: bool,
    pub output: OutputLimits,
    pub edit_cadence: Duration,
    pub upload_bytes: Option<u64>,
}
impl Capabilities {
    /// Safe default for a room whose transport or permissions are not known yet.
    pub fn unavailable() -> Self {
        Self {
            visibility: MessageVisibility::MentionsOnly,
            history: HistoryAccess::None,
            decryption_available: false,
            replies: false,
            threads: false,
            send: false,
            react: false,
            observe_reactions: false,
            edit: false,
            delete: false,
            typing: false,
            nickname: NicknameScope::None,
            rich_cards: false,
            components: false,
            modals: false,
            native_commands: false,
            private_results: false,
            output: OutputLimits::DISCORD,
            edit_cadence: Duration::from_millis(1200),
            upload_bytes: None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresentationPolicy {
    Normal,
    Quiet,
}

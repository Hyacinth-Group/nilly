use super::read_models::{HelpView, MindView, SettingsView, StatusView};
use crate::platform::types::{ConnectionId, RoomRef, UserKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    Status,
    Help,
    Mind,
    Settings,
    Reset,
    News,
    Problem,
    Wander,
    Prank,
    Hack,
    Agents,
    Wake,
    Sleep,
    Thinking,
    Model,
    Debug,
}
impl ActionId {
    pub const ALL: [Self; 16] = [
        Self::Status,
        Self::Help,
        Self::Mind,
        Self::Settings,
        Self::Reset,
        Self::News,
        Self::Problem,
        Self::Wander,
        Self::Prank,
        Self::Hack,
        Self::Agents,
        Self::Wake,
        Self::Sleep,
        Self::Thinking,
        Self::Model,
        Self::Debug,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Help => "help",
            Self::Mind => "mind",
            Self::Settings => "settings",
            Self::Reset => "reset",
            Self::News => "news",
            Self::Problem => "problem",
            Self::Wander => "wander",
            Self::Prank => "prank",
            Self::Hack => "hack",
            Self::Agents => "agents",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
            Self::Thinking => "thinking",
            Self::Model => "model",
            Self::Debug => "debug",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingSetting {
    Show,
    Hide,
    Silent,
    Off,
}
impl ThinkingSetting {
    pub(crate) fn core(self) -> crate::ThinkingMode {
        match self {
            Self::Show => crate::ThinkingMode::Show,
            Self::Hide => crate::ThinkingMode::Hide,
            Self::Silent => crate::ThinkingMode::Silent,
            Self::Off => crate::ThinkingMode::Off,
        }
    }
    pub fn file_value(self) -> &'static str {
        self.core().file_value()
    }
    pub fn label(self) -> &'static str {
        self.core().label()
    }
}
impl From<crate::ThinkingMode> for ThinkingSetting {
    fn from(value: crate::ThinkingMode) -> Self {
        match value {
            crate::ThinkingMode::Show => Self::Show,
            crate::ThinkingMode::Hide => Self::Hide,
            crate::ThinkingMode::Silent => Self::Silent,
            crate::ThinkingMode::Off => Self::Off,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugSetting {
    On,
    Off,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionArgs {
    Status {},
    Help {},
    Mind {
        #[serde(default = "super::schema::default_mind_test")]
        test: bool,
    },
    Settings {},
    Reset {
        #[serde(default = "super::schema::default_reset_all")]
        all: bool,
    },
    News {},
    Problem {},
    Wander {},
    Prank {},
    Hack {},
    Agents {},
    Wake {},
    Sleep {
        #[serde(default = "super::schema::default_sleep_hours")]
        hours: i64,
    },
    Thinking {
        mode: Option<ThinkingSetting>,
    },
    Model {
        id: Option<String>,
    },
    Debug {
        setting: Option<DebugSetting>,
    },
}
impl ActionArgs {
    pub fn id(&self) -> ActionId {
        match self {
            Self::Status {} => ActionId::Status,
            Self::Help {} => ActionId::Help,
            Self::Mind { .. } => ActionId::Mind,
            Self::Settings {} => ActionId::Settings,
            Self::Reset { .. } => ActionId::Reset,
            Self::News {} => ActionId::News,
            Self::Problem {} => ActionId::Problem,
            Self::Wander {} => ActionId::Wander,
            Self::Prank {} => ActionId::Prank,
            Self::Hack {} => ActionId::Hack,
            Self::Agents {} => ActionId::Agents,
            Self::Wake {} => ActionId::Wake,
            Self::Sleep { .. } => ActionId::Sleep,
            Self::Thinking { .. } => ActionId::Thinking,
            Self::Model { .. } => ActionId::Model,
            Self::Debug { .. } => ActionId::Debug,
        }
    }
    pub fn requires_room(&self) -> bool {
        matches!(
            self,
            Self::Mind { test: true }
                | Self::Reset { all: false }
                | Self::News {}
                | Self::Problem {}
                | Self::Prank {}
                | Self::Hack {}
        )
    }
}
/// Only an authenticated local operator boundary may construct LocalOperator.
#[derive(Debug, Clone)]
pub enum Actor {
    LocalOperator,
    User(UserKey),
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionTarget {
    pub connection: Option<ConnectionId>,
    pub room: Option<RoomRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActionValue {
    Boolean(bool),
    Integer(i64),
    String(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Private,
    Public,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum ActionResult {
    Status(StatusView),
    Help(HelpView),
    Mind(MindView),
    Settings(SettingsView),
    Ack { title: String, description: String },
}
impl ActionResult {
    /// Native administration has always replied privately, including acknowledgements.
    pub fn visibility(&self) -> Visibility {
        Visibility::Private
    }
}
#[derive(Debug)]
pub enum ActionError {
    InvalidArgument { key: String, reason: &'static str },
    UnknownAction,
    PermissionDenied,
    MissingConnection,
    MissingRoom,
    UnboundRoom,
    UnsupportedRoute,
    PrivateSurfaceRequired,
    Memory(crate::BotError),
    Platform(crate::platform::PlatformError),
}
impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument { key, reason } => write!(f, "invalid argument {key}: {reason}"),
            Self::UnknownAction => f.write_str("unknown action"),
            Self::PermissionDenied => f.write_str("administration permission denied"),
            Self::MissingConnection => f.write_str("administration requires a connection"),
            Self::MissingRoom => f.write_str("action requires a room"),
            Self::UnboundRoom => f.write_str("room is not bound to this connection"),
            Self::UnsupportedRoute => {
                f.write_str("action cannot address a thread through a room-only route")
            }
            Self::PrivateSurfaceRequired => {
                f.write_str("private administration response surface required")
            }
            Self::Memory(error) => write!(f, "memory: {error}"),
            Self::Platform(error) => write!(f, "platform: {error}"),
        }
    }
}
impl std::error::Error for ActionError {}

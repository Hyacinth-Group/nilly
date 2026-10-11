//! Transport-neutral administration. Native registration, interaction deadlines and
//! card builders live in platform adapters; every entry surface shares this schema.
use crate::{now_unix, sleep, Bot};
use serde::Deserialize;

pub mod parser;
pub mod read_models;
pub mod render;
pub mod schema;
mod service;
pub mod types;

pub(crate) use service::AdminService;
pub use types::{
    ActionArgs, ActionError, ActionId, ActionResult, ActionTarget, ActionValue, Actor,
    DebugSetting, ThinkingSetting, Visibility,
};

include!("command/remaining.rs");

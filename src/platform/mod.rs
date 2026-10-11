//! Platform SDKs decode into these domain values. Personality/model logic does not
//! belong to an adapter; credential/session IDs are distinct from durable identities.
pub mod capabilities;
pub mod discord;
pub mod interface;
pub mod keys;
pub mod outgoing;
pub mod registry;
pub mod types;

pub use interface::{Adapter, Platform, PlatformError, PlatformResult};
pub use registry::ConnectionRegistry;

//! Platform SDKs decode into these domain values. Personality/model logic does not
//! belong to an adapter; credential/session IDs are distinct from durable identities.
pub mod capabilities;
pub mod discord;
pub mod types;

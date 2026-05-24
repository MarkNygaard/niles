//! Error types for niles-tools.

use thiserror::Error;

/// Errors surfaced by the tool registry and built-in tools.
#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    #[error("invalid arguments for tool '{tool}': {reason}")]
    InvalidArgs { tool: String, reason: String },

    #[error("MQTT publish failed: {0}")]
    Mqtt(#[from] niles_mqtt::Error),

    #[error("device not found: {id}")]
    DeviceNotFound { id: String },

    #[error("room not found: {name}")]
    RoomNotFound { name: String },

    #[error("internal tool error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

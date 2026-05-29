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

    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("device not found: {id}")]
    DeviceNotFound { id: String },

    #[error("device {id} is a {class:?}; set_device only accepts lights")]
    WrongDeviceClass {
        id: String,
        class: niles_core::DeviceClass,
    },

    #[error("room not found: {name}")]
    RoomNotFound { name: String },

    #[error("internal tool error: {0}")]
    Internal(String),

    #[error("memory error: {0}")]
    Memory(String),

    #[error("skill error: {0}")]
    Skill(String),

    #[error("weather error: {0}")]
    Weather(String),
}

pub type Result<T> = std::result::Result<T, Error>;

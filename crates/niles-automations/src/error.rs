//! Error types for niles-automations.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid automation id '{id}': {reason}")]
    InvalidId { id: String, reason: String },

    #[error("automation '{id}' has no actions")]
    NoActions { id: String },

    #[error("automation '{id}': invalid brightness {value} (must be 0–100)")]
    InvalidBrightness { id: String, value: u8 },

    #[error("automation '{id}': invalid kelvin {value} (must be 2000–6500)")]
    InvalidKelvin { id: String, value: u16 },

    #[error("automation '{id}': invalid time '{value}': {source}")]
    InvalidTime {
        id: String,
        value: String,
        #[source]
        source: chrono::ParseError,
    },

    #[error("automation '{id}': invalid priority '{value}'")]
    InvalidPriority { id: String, value: String },

    #[error("automation '{id}': invalid device id '{value}': {source}")]
    InvalidDeviceId {
        id: String,
        value: String,
        #[source]
        source: niles_core::Error,
    },

    #[error("automation '{id}': invalid room '{value}': {source}")]
    InvalidRoom {
        id: String,
        value: String,
        #[source]
        source: niles_core::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

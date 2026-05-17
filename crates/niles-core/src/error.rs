//! Error types for niles-core.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid {kind} name: {reason}")]
    InvalidName { kind: &'static str, reason: String },

    #[error("invalid device identifier: {0}")]
    InvalidDeviceId(String),

    #[error("device not found: {0}")]
    DeviceNotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;

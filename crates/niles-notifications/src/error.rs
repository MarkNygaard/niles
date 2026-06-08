//! Error types for niles-notifications.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("notification log I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("notification log serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid notification configuration: {0}")]
    InvalidConfig(String),
    #[error("delivery failed: {0}")]
    DeliveryFailed(String),
}
pub type Result<T> = std::result::Result<T, Error>;

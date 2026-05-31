//! Error types for niles-notifications.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid notification configuration: {0}")]
    InvalidConfig(String),

    #[error("delivery failed: {0}")]
    DeliveryFailed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

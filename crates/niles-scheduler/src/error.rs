//! Error types for niles-scheduler.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid time-of-day: {reason}")]
    InvalidTime { reason: String },

    #[error("invalid curve config: {reason}")]
    InvalidConfig { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

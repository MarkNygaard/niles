//! Error types for niles-wyoming.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Wyoming I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Wyoming JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Wyoming frame error: {reason}")]
    Frame { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SendError {
    #[error("peer not connected")]
    NotConnected,
}

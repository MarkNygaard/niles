//! Error types for the skills crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("skill `{name}` already exists")]
    AlreadyExists { name: String },

    #[error("invalid skill name `{name}`: {reason}")]
    InvalidName { name: String, reason: String },

    #[error("security scan failed: {reason}")]
    ScanFailed { reason: String },

    #[error("content too large: {reason}")]
    TooLarge { reason: String },

    #[error("skill `{name}` is pinned")]
    Pinned { name: String },

    #[error("skill `{name}` not found")]
    NotFound { name: String },

    #[error("skill `{name}` was folded into `{into}`")]
    Absorbed { name: String, into: String },
}

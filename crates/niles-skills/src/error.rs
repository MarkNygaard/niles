//! Error types for the skills crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("YAML serialization failed: {0}")]
    Yaml(#[from] serde_yaml::Error),

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

    #[error("store is locked by another process")]
    Locked,
}

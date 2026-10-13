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

    #[error(
        "the text to replace is not in skill `{name}`; view_skill it and copy the text exactly"
    )]
    EditNotFound { name: String },

    #[error(
        "the text to replace appears {count} times in skill `{name}`; include more of it so it appears once"
    )]
    EditAmbiguous { name: String, count: usize },

    #[error("skill `{name}` has no change to undo")]
    NothingToUndo { name: String },
}

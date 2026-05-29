//! Error types for the memory crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("file is locked by another process")]
    Locked,

    #[error("security scan failed: {reason}")]
    ScanFailed { reason: String },

    #[error("{target} memory exceeds char budget: {actual} > {limit}")]
    OverBudget {
        target: &'static str,
        limit: usize,
        actual: usize,
    },

    #[error("no entry in {target} memory matches the given text")]
    NotFound { target: &'static str },

    #[error("multiple entries in {target} memory match the given text; be more specific")]
    Ambiguous { target: &'static str },

    #[error("search text must not be empty")]
    EmptySearch,

    #[error("entry content must not contain a standalone '§' delimiter line")]
    InvalidDelimiter,
}

//! Error types for the history crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid date range: {reason}")]
    InvalidDateRange { reason: String },

    /// The store behind a history failed: `kind` is which history.
    #[error("{kind} history: {reason}")]
    Storage { kind: &'static str, reason: String },
}

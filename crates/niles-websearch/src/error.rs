//! Error types for the websearch crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to search API failed: {error}")]
    Http {
        #[from]
        error: reqwest::Error,
    },

    #[error("upstream returned HTTP {status}: {body}")]
    BadStatus { status: u16, body: String },

    #[error("parse error: {reason}")]
    Parse { reason: String },

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

    /// A page Niles will not fetch: not the public internet.
    #[error("refused: {reason}")]
    Refused { reason: String },

    /// A page that could not be turned into text.
    #[error("could not read it: {reason}")]
    Unreadable { reason: String },
}

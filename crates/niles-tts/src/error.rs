//! Error types for the TTS crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid synthesis input: {0}")]
    InvalidInput(String),

    #[error("HTTP request to TTS provider failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("TTS provider returned status {status}: {body}")]
    Provider { status: u16, body: String },

    #[error("local I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

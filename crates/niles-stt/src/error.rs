//! Error types for the STT crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to STT provider failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("STT provider returned status {status}: {body}")]
    Provider { status: u16, body: String },

    #[error("failed to parse STT provider response: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("PCM->WAV conversion failed: {reason}")]
    Wav { reason: String },

    #[error("local I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

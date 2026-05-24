//! Error types for the LLM crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to LLM provider failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("LLM provider returned status {status}: {body}")]
    Provider { status: u16, body: String },

    #[error("failed to parse LLM provider response: {0}")]
    Decode(#[from] serde_json::Error),
}

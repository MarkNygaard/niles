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

    /// Both of Piper's synthesis paths answered "no such page" or "wrong
    /// method": the address is most likely not a Piper HTTP server's base.
    #[error(
        "Piper at {base_url} accepted neither POST {base_url}/synthesize (status {synthesize}) \
         nor POST {base_url}/ (status {root}); the TTS address should be Piper's base address, \
         like http://piper:5000"
    )]
    NoSynthesisEndpoint {
        base_url: String,
        synthesize: u16,
        root: u16,
    },

    #[error("local I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

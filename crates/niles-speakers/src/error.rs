//! Error types for the speakers crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to Sonos failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("SOAP fault ({code}): {reason}")]
    SoapFault { code: String, reason: String },

    #[error("malformed SOAP response: {reason}")]
    ParseResponse { reason: String },
}

//! Error types for the weather crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to weather API failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("upstream returned HTTP {status}: {body}")]
    BadStatus { status: u16, body: String },

    #[error("parse error: {reason}")]
    Parse { reason: String },

    #[error("geocoding returned no results for '{query}'")]
    GeocodeEmpty { query: String },

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },
}

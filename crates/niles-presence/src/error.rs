//! Error types for the presence crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("upstream returned HTTP {status}: {body}")]
    BadStatus { status: u16, body: String },

    #[error("parse error: {reason}")]
    Parse { reason: String },

    #[error("auth error: {reason}")]
    Auth { reason: String },

    #[error("auth unsupported: {reason}")]
    AuthUnsupported { reason: String },

    #[error("missing env var: {name}")]
    MissingEnv { name: String },
}

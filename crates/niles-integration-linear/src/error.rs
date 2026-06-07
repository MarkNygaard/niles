//! Error types for the Linear integration crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request to Linear API failed: {source}")]
    Http {
        #[from]
        source: reqwest::Error,
    },

    #[error("upstream returned HTTP {status}: {body}")]
    BadStatus { status: u16, body: String },

    #[error("parse error: {reason}")]
    Parse { reason: String },

    #[error("Linear GraphQL error: {reason}")]
    Api { reason: String },

    #[error("could not resolve {kind} '{name}' in Linear workspace")]
    Resolve { kind: &'static str, name: String },
}

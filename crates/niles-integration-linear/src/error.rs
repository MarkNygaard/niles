//! Error types for the Linear integration crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("parse error: {reason}")]
    Parse { reason: String },
}

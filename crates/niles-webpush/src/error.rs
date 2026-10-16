use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },
    #[error("push storage: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, Error>;

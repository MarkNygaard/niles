use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("there is no item {id} on the list")]
    NotFound { id: u64 },
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

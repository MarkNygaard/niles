//! Errors from checking, keeping and reading a person's schedules.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A value a person typed didn't pass its checks. The reason never
    /// quotes what was typed.
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },
    /// The store behind the schedules failed.
    #[error("schedules' storage: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, Error>;

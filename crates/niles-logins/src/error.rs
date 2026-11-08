//! Errors from reading, writing and checking a saved login.

/// Why a saved login couldn't be read, written or accepted.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A person-typed value didn't pass its checks. The reason says what
    /// was wrong without quoting what was typed: never a username, a
    /// password or the site address as it came, so neither an error
    /// message, a log line nor a response can carry one away.
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },
    /// The store behind the logins failed.
    #[error("saved logins' storage: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, Error>;

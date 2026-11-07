//! Why a check failed, as a person can read it.

use chrono::{DateTime, Utc};

/// Why a check could not finish. Every `Display` is a user-facing
/// reason; none of them may contain the pull token.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The HTTP client itself could not be built.
    #[error("the HTTP client couldn't be built: {reason}")]
    Client { reason: String },
    /// No answer at all: DNS, a refused connection, a dropped body.
    #[error("{what} couldn't be reached: {reason}")]
    Unreachable { what: &'static str, reason: String },
    /// An answer, but not a success.
    #[error("{what} answered HTTP {status}")]
    Status { what: &'static str, status: u16 },
    /// GitHub's anonymous limit, until `until`.
    #[error("{}", rate_limited_message(.what, .until))]
    RateLimited {
        what: &'static str,
        until: Option<DateTime<Utc>>,
    },
    /// One of the two commits is not on GitHub, so what changed cannot
    /// be listed.
    #[error("GitHub doesn't know one of the two commits, so what changed can't be listed")]
    UnknownCommit,
    /// A body larger than its cap.
    #[error("{what} is larger than {cap} bytes")]
    TooLarge { what: &'static str, cap: usize },
    /// A body that does not read.
    #[error("{what} couldn't be read: {reason}")]
    Malformed { what: &'static str, reason: String },
    /// No answer within the timeout.
    #[error("{what} didn't answer within {after_secs} seconds")]
    Timeout { what: &'static str, after_secs: u64 },
}

pub type Result<T> = std::result::Result<T, Error>;

/// "… until HH:MM UTC", or "for now" when the reset time is unknown.
fn rate_limited_message(what: &str, until: &Option<DateTime<Utc>>) -> String {
    match until {
        Some(at) => format!(
            "{what} takes no more anonymous requests from this address until {} UTC",
            at.format("%H:%M")
        ),
        None => format!("{what} takes no more anonymous requests from this address for now"),
    }
}

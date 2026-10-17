//! Errors a podcast's making can meet, in a person's words.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Kokoro could not say a line, or answered something that is not
    /// MP3.
    #[error("{reason}")]
    Speech { reason: String },
    /// The bucket refused, or was unreachable. `action` is "reach",
    /// "upload", "read" or "delete" — which part failed.
    #[error("{reason}")]
    Storage {
        action: &'static str,
        reason: String,
    },
    /// Audio that could not be parsed or joined.
    #[error("{reason}")]
    Audio { reason: String },
    /// The add-on is not set up; `missing` names what is still needed.
    #[error("the Podcasts add-on still needs its {}", missing.join(", "))]
    NotSetUp { missing: Vec<&'static str> },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

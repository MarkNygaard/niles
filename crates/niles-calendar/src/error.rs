use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The request never got an answer.
    #[error("Microsoft did not answer: {reason}")]
    Http { reason: String },

    /// Microsoft answered, and said no.
    #[error("Microsoft refused ({status}): {reason}")]
    Microsoft { status: u16, reason: String },

    /// The sign-in behind a connection no longer works — revoked,
    /// expired, or the password changed. Only connecting again helps.
    #[error("the {name} calendar has to be connected again")]
    Reconnect { name: String },

    /// Writing to a calendar connected read-only.
    #[error("the {name} calendar is connected read only")]
    ReadOnly { name: String },

    #[error("no calendar connection called {name:?}")]
    NoSuchConnection { name: String },

    #[error("no event {reference:?}; look the events up again first")]
    NoSuchEvent { reference: String },

    /// A question about the store, not about Microsoft.
    #[error("calendar storage: {reason}")]
    Storage { reason: String },

    #[error("{reason}")]
    Invalid { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

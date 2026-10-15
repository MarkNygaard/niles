use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },
    #[error("training storage: {0}")]
    Storage(String),
    #[error("Strava is not connected for this person")]
    NotConnected,
    /// Strava no longer accepts the person's tokens: they revoked Niles
    /// on strava.com, or the app's secret changed.
    #[error("Strava no longer accepts this connection; connect again")]
    Unauthorized,
    /// The person's app has spent its request budget until then.
    #[error("Strava's request limit is reached until {until}")]
    RateLimited { until: DateTime<Utc> },
    #[error("a sync is already running")]
    Busy,
    /// Strava answered with something else, or did not answer.
    #[error("Strava: {message}")]
    Strava {
        status: Option<u16>,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

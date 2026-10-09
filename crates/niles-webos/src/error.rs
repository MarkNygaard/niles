use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Nothing answered: off, asleep without Quick Start+, or a firewall
    /// between Niles and the TV.
    #[error("the TV at {host} did not answer: {reason}")]
    Unreachable { host: String, reason: String },

    /// The TV answered and refused: no pairing, or a pairing it forgot.
    #[error("the TV refused Niles: {0}")]
    Refused(String),

    /// The TV turned down one request.
    #[error("the TV could not {request}: {reason}")]
    Request { request: String, reason: String },

    #[error("the TV's answer made no sense: {0}")]
    Protocol(String),

    #[error("waking the TV: {0}")]
    Wake(#[from] std::io::Error),

    #[error("{0} is not a MAC address")]
    BadMac(String),
}

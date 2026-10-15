use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Nothing answered: DNS, TLS, a refused connection, a timeout.
    #[error("could not reach the server: {0}")]
    Unreachable(String),

    /// The server answered, but not with success. `401` and `403` are the
    /// common ones, and mean the key.
    #[error("{status}: {reason}")]
    Refused { status: u16, reason: String },

    /// A reply that is not JSON-RPC, or not the reply to what was asked.
    #[error("the server's reply made no sense: {0}")]
    Protocol(String),

    /// A JSON-RPC error object.
    #[error("the server said: {message} ({code})")]
    Rpc { code: i64, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

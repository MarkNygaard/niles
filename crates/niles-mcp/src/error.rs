use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Nothing answered: DNS, TLS, a refused connection, a timeout.
    #[error("could not reach the server: {0}")]
    Unreachable(String),

    /// The server answered, but not with success. `403` is the common
    /// one, and means the key.
    #[error("{status}: {reason}")]
    Refused { status: u16, reason: String },

    /// `401`: the server wants a sign-in, or a key it accepts. When it
    /// says where its sign-in is described (RFC 9728's
    /// `resource_metadata`), that address comes along.
    #[error("401: {reason}")]
    Unauthorized {
        reason: String,
        resource_metadata: Option<String>,
    },

    /// Signing in to a server didn't work; the reason names no token.
    #[error("sign-in: {0}")]
    SignIn(String),

    /// A value a person typed didn't pass its checks. The reason never
    /// quotes what was typed.
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: &'static str, reason: String },

    /// The store behind a person's servers failed.
    #[error("personal servers' storage: {0}")]
    Storage(String),

    /// An address a person's server, or its sign-in, may not use.
    #[error("not allowed: {0}")]
    NotPublic(String),

    /// A reply that is not JSON-RPC, or not the reply to what was asked.
    #[error("the server's reply made no sense: {0}")]
    Protocol(String),

    /// A JSON-RPC error object.
    #[error("the server said: {message} ({code})")]
    Rpc { code: i64, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

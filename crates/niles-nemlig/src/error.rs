use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The username or password was refused.
    #[error("nemlig.com refused the login: {reason}")]
    Login { reason: String },
    /// The site answered, but not with what was asked for.
    #[error("nemlig.com answered {status}: {reason}")]
    Api { status: u16, reason: String },
    #[error("could not reach nemlig.com: {0}")]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

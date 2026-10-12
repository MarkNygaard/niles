use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The username or password was refused.
    #[error("nemlig.com refused the login: {reason}")]
    Login { reason: String },
    /// The site answered, but not with what was asked for.
    #[error("nemlig.com answered {status}: {reason}")]
    Api { status: u16, reason: String },
    /// A success that was not the JSON asked for — a web page where an
    /// answer should be, which is what nemlig sends a session it has
    /// ended without saying so. `seen` is its content type and title.
    #[error("nemlig.com answered {status} with {seen}, not an answer Niles could read")]
    Unreadable { status: u16, seen: String },
    #[error("could not reach nemlig.com: {0}")]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

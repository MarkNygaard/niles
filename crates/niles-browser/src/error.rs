//! Errors of the browser crate. None of them carries a page's content.

/// An error talking to the browser, its proxy, or shaping a picture.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// the browser at {address} can't be reached: {reason}
    #[error("the browser at {address} can't be reached: {reason}")]
    Unreachable { address: String, reason: String },

    /// the browser refused {method}: {reason}. `reason` is CDP's `message`
    /// and `code`, never `data` and never the parameters.
    #[error("the browser refused {method}: {reason}")]
    Protocol {
        method: &'static str,
        reason: String,
    },

    /// the connection to the browser closed
    #[error("the connection to the browser closed")]
    Closed,

    /// the browser didn't answer {method} in time
    #[error("the browser didn't answer {method} in time")]
    Timeout { method: &'static str },

    /// the page didn't open: {reason}
    #[error("the page didn't open: {reason}")]
    Navigation { reason: String },

    /// {method} answered in a shape that wasn't expected: {reason}. A shape
    /// is logged, never a value.
    #[error("{method} answered in a shape that wasn't expected: {reason}")]
    Answer {
        method: &'static str,
        reason: String,
    },

    /// the picture couldn't be made: {reason}
    #[error("the picture couldn't be made: {reason}")]
    Picture { reason: String },

    /// the proxy couldn't listen at {address}: {reason}
    #[error("the proxy couldn't listen at {address}: {reason}")]
    Listen { address: String, reason: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

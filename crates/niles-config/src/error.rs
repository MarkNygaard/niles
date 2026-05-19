//! Error types for niles-config.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not read config file: {0}")]
    Read(#[from] std::io::Error),

    #[error("could not parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("invalid [{section}] section: {reason}")]
    InvalidSection {
        section: &'static str,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

//! Error types for the capabilities crate.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("directory walk or file read failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("capability directory `{dir}` does not contain a SKILL.md file")]
    MissingSkillFile { dir: PathBuf },

    #[error("invalid frontmatter in `{dir}`: {reason}")]
    Frontmatter { dir: PathBuf, reason: String },

    #[error("SKILL.md in `{dir}` has frontmatter but no body")]
    BodyMissing { dir: PathBuf },

    #[error("duplicate capability name `{name}` found in `{}` and `{}`", first.display(), second.display())]
    DuplicateName {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
}

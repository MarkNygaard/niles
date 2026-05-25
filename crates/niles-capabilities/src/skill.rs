//! Capability data model.

use std::path::PathBuf;

use serde::Deserialize;

/// Parsed YAML frontmatter from a `SKILL.md` file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CapabilityMetadata {
    /// Kebab-case identifier, e.g. `lights-control`.
    pub name: String,

    /// One-line summary shown in listings.
    pub description: String,

    /// Semver-ish version string.
    pub version: String,

    /// Names of other capabilities that should be loaded first.
    #[serde(default)]
    pub prerequisites: Vec<String>,
}

/// A fully loaded capability: metadata + markdown body + source directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    pub metadata: CapabilityMetadata,
    pub body: String,
    pub dir: PathBuf,
}

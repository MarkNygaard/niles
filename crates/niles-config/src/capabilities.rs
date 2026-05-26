//! Capability reference loader configuration.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// `[capabilities]` section of the config file.
///
/// Optional. If absent (or `directory` is `None`), niles starts with
/// no capabilities loaded — the LLM tool surface keeps the device
/// tools and no `look_up_capability`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesConfig {
    /// Path to the directory containing capability subdirectories
    /// (one per skill, each with a `SKILL.md`). If `None`, the
    /// loader is not built and `look_up_capability` is not
    /// registered.
    pub directory: Option<PathBuf>,
}

impl CapabilitiesConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(dir) = &self.directory
            && dir.as_os_str().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "capabilities",
                reason: "directory must not be empty if present".into(),
            });
        }
        Ok(())
    }
}

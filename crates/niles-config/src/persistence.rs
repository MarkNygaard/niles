//! Persistence configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// `[persistence]` section of the config file.
///
/// Optional. If absent (or `directory` is `None`), niles runs
/// fully in-memory — timers, scenes, and morning claims do not
/// survive a process restart.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PersistenceConfig {
    /// Path to the directory where niles writes its state files
    /// (`timers.json`, `scenes.json`, `morning_claims.json`).
    /// If `None`, persistence is disabled.
    pub directory: Option<PathBuf>,
}

impl PersistenceConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(dir) = &self.directory
            && dir.as_os_str().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "persistence",
                reason: "directory must not be empty if present".into(),
            });
        }
        Ok(())
    }
}

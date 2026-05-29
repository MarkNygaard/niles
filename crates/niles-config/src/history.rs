//! History configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

fn default_retention_days() -> u32 {
    14
}

/// `[history]` section of the config file.
///
/// Optional. If absent (or `directory` is `None`), voice command
/// history is disabled — no JSONL files are written and the
/// `query_command_history` tool returns an empty array.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryConfig {
    /// Path to the directory where niles writes command history
    /// (`<dir>/commands/YYYY-MM-DD.jsonl`). If `None`, history is
    /// disabled.
    pub directory: Option<PathBuf>,
    /// How many days of history to keep. Files older than this are
    /// pruned at startup. Default: 14.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            directory: None,
            retention_days: default_retention_days(),
        }
    }
}

impl HistoryConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(dir) = &self.directory
            && dir.as_os_str().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "history",
                reason: "directory must not be empty if present".into(),
            });
        }
        if !(1..=365).contains(&self.retention_days) {
            return Err(Error::InvalidSection {
                section: "history",
                reason: format!(
                    "retention_days must be between 1 and 365, got {}",
                    self.retention_days
                ),
            });
        }
        Ok(())
    }
}

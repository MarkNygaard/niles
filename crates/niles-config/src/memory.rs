//! Memory configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

fn default_user_char_limit() -> usize {
    1375
}

fn default_agent_char_limit() -> usize {
    2200
}

/// `[memory]` section of the config file.
///
/// Optional. If absent (or `directory` is `None`), persistent memory
/// is disabled — no markdown files are written and the `memory` tool
/// is not registered.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// Path to the directory where niles writes `USER.md` and
    /// `MEMORY.md`. If `None`, memory is disabled.
    pub directory: Option<PathBuf>,
    /// Maximum characters for `USER.md`. Default: 1375.
    #[serde(default = "default_user_char_limit")]
    pub user_char_limit: usize,
    /// Maximum characters for `MEMORY.md`. Default: 2200.
    #[serde(default = "default_agent_char_limit")]
    pub agent_char_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            directory: None,
            user_char_limit: default_user_char_limit(),
            agent_char_limit: default_agent_char_limit(),
        }
    }
}

impl MemoryConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(dir) = &self.directory
            && dir.as_os_str().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "memory",
                reason: "directory must not be empty if present".into(),
            });
        }
        if self.user_char_limit == 0 {
            return Err(Error::InvalidSection {
                section: "memory",
                reason: "user_char_limit must be > 0".into(),
            });
        }
        if self.agent_char_limit == 0 {
            return Err(Error::InvalidSection {
                section: "memory",
                reason: "agent_char_limit must be > 0".into(),
            });
        }
        Ok(())
    }
}

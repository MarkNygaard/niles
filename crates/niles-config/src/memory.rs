//! Memory configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_user_char_limit() -> usize {
    1375
}

fn default_agent_char_limit() -> usize {
    2200
}

/// `[memory]` section of the config file.
///
/// Optional, and only sizes. Memory is always on, and kept in the
/// database when there is one; it used to need a `directory` here, and
/// without it Niles forgot everything it was told.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// Maximum characters of household facts. Default: 1375.
    #[serde(default = "default_user_char_limit")]
    pub user_char_limit: usize,
    /// Maximum characters of Niles's own learnings. Default: 2200.
    #[serde(default = "default_agent_char_limit")]
    pub agent_char_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            user_char_limit: default_user_char_limit(),
            agent_char_limit: default_agent_char_limit(),
        }
    }
}

impl MemoryConfig {
    pub fn validate(&self) -> Result<()> {
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

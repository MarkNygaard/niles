//! Skills configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

fn default_skill_max_chars() -> usize {
    100_000
}

fn default_supporting_file_max_bytes() -> usize {
    1_048_576 // 1 MiB
}

/// `[skills]` section of the config file.
///
/// Optional. If absent (or `directory` is `None`), the writable skill
/// store is disabled.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsConfig {
    /// Path to the directory where niles writes skills.
    /// If `None`, the skill store is disabled.
    pub directory: Option<PathBuf>,
    /// Maximum number of characters in a skill body.
    #[serde(default = "default_skill_max_chars")]
    pub skill_max_chars: usize,
    /// Maximum size in bytes for supporting files inside a skill directory.
    #[serde(default = "default_supporting_file_max_bytes")]
    pub supporting_file_max_bytes: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            directory: None,
            skill_max_chars: default_skill_max_chars(),
            supporting_file_max_bytes: default_supporting_file_max_bytes(),
        }
    }
}

impl SkillsConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(dir) = &self.directory
            && dir.as_os_str().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "directory must not be empty if present".into(),
            });
        }
        if self.skill_max_chars == 0 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "skill_max_chars must be > 0".into(),
            });
        }
        if self.supporting_file_max_bytes == 0 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "supporting_file_max_bytes must be > 0".into(),
            });
        }
        Ok(())
    }
}

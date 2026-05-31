//! Integrations configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_timeout_seconds() -> u64 {
    15
}

/// Top-level `[integrations]` section of the config file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub archon: Option<ArchonConfigDto>,
}

/// `[integrations.archon]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchonConfigDto {
    pub base_url: String,
    pub codebase_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl IntegrationsConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(archon) = &self.archon {
            archon.validate()?;
        }
        Ok(())
    }
}

impl ArchonConfigDto {
    pub fn validate(&self) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.archon",
                reason: "base_url must not be empty".into(),
            });
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(Error::InvalidSection {
                section: "integrations.archon",
                reason: format!(
                    "base_url '{}' must start with http:// or https://",
                    self.base_url
                ),
            });
        }
        if self.codebase_id.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.archon",
                reason: "codebase_id must not be empty".into(),
            });
        }
        if let Some(cwd) = &self.cwd
            && cwd.trim().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "integrations.archon",
                reason: "cwd must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 120 {
            return Err(Error::InvalidSection {
                section: "integrations.archon",
                reason: "timeout_seconds must be between 1 and 120".into(),
            });
        }
        Ok(())
    }
}

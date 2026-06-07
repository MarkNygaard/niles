//! Integrations configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_timeout_seconds() -> u64 {
    15
}

fn default_trigger_label() -> String {
    "AI Eligible".into()
}

fn default_todo_state() -> String {
    "Todo".into()
}

/// Top-level `[integrations]` section of the config file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub linear: Option<LinearConfigDto>,
}

/// `[integrations.linear]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearConfigDto {
    pub api_key_env: String,
    pub team: String,
    #[serde(default = "default_trigger_label")]
    pub trigger_label: String,
    #[serde(default = "default_todo_state")]
    pub todo_state: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl IntegrationsConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(linear) = &self.linear {
            linear.validate()?;
        }
        Ok(())
    }
}

impl LinearConfigDto {
    pub fn validate(&self) -> Result<()> {
        if self.api_key_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.linear",
                reason: "api_key_env must not be empty".into(),
            });
        }
        if self.team.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.linear",
                reason: "team must not be empty".into(),
            });
        }
        if self.trigger_label.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.linear",
                reason: "trigger_label must not be empty".into(),
            });
        }
        if self.todo_state.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.linear",
                reason: "todo_state must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 120 {
            return Err(Error::InvalidSection {
                section: "integrations.linear",
                reason: "timeout_seconds must be between 1 and 120".into(),
            });
        }
        Ok(())
    }

    pub fn resolve_api_key(&self) -> Result<String> {
        std::env::var(&self.api_key_env).map_err(|_| Error::InvalidSection {
            section: "integrations.linear",
            reason: format!("env var {} is not set", self.api_key_env),
        })
    }
}

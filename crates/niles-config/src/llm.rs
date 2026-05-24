//! Tier 1 LLM provider configuration.
//!
//! Same secrets pattern as `[stt]`: the TOML carries the *name* of
//! the env var that holds the API key. The runtime resolves it at
//! startup so secrets stay out of the config file.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[llm]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    /// Name of the env var holding the provider API key
    /// (e.g. `"GROQ_API_KEY"`).
    pub api_key_env: String,
    /// Provider base URL. Defaults to Groq's hosted endpoint.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Model identifier passed to the provider.
    #[serde(default = "default_model")]
    pub model: String,
    /// Provider request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_seconds: u64,
}

fn default_base_url() -> String {
    "https://api.groq.com/openai/v1".into()
}

fn default_model() -> String {
    "openai/gpt-oss-20b".into()
}

fn default_timeout_secs() -> u64 {
    30
}

impl LlmConfig {
    pub fn validate(&self) -> Result<()> {
        if self.api_key_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "llm",
                reason: "api_key_env must not be empty".into(),
            });
        }
        if self.base_url.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "llm",
                reason: "base_url must not be empty".into(),
            });
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(Error::InvalidSection {
                section: "llm",
                reason: format!(
                    "base_url '{}' must start with http:// or https://",
                    self.base_url
                ),
            });
        }
        if self.model.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "llm",
                reason: "model must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 {
            return Err(Error::InvalidSection {
                section: "llm",
                reason: "timeout_seconds must be > 0".into(),
            });
        }
        Ok(())
    }

    /// Read the API key from the env var named by `api_key_env`.
    /// Returns an `InvalidSection` error if it's unset.
    pub fn resolve_api_key(&self) -> Result<String> {
        std::env::var(&self.api_key_env).map_err(|_| Error::InvalidSection {
            section: "llm",
            reason: format!("env var {} is not set", self.api_key_env),
        })
    }
}

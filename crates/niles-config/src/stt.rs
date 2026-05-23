//! Speech-to-text provider configuration.
//!
//! Same secrets pattern as `[mqtt]`: the TOML carries the *name* of
//! the env var that holds the API key. The runtime resolves it at
//! startup so secrets stay out of the config file.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[stt]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SttConfig {
    /// Name of the env var holding the provider API key
    /// (e.g. `"GROQ_API_KEY"`).
    pub api_key_env: String,
    /// Provider base URL. Defaults to Groq's hosted endpoint.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Model identifier passed to the provider.
    #[serde(default = "default_model")]
    pub model: String,
    /// Optional ISO-639-1 language hint (e.g. `"en"`). `None` lets
    /// Whisper auto-detect.
    #[serde(default)]
    pub language: Option<String>,
    /// Provider request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_seconds: u64,
}

fn default_base_url() -> String {
    "https://api.groq.com/openai/v1".into()
}

fn default_model() -> String {
    "whisper-large-v3-turbo".into()
}

fn default_timeout_secs() -> u64 {
    30
}

impl SttConfig {
    pub fn validate(&self) -> Result<()> {
        if self.api_key_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "api_key_env must not be empty".into(),
            });
        }
        if self.base_url.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "base_url must not be empty".into(),
            });
        }
        if self.model.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "model must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "timeout_seconds must be > 0".into(),
            });
        }
        if let Some(lang) = &self.language
            && lang.trim().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "language must not be empty when set (omit the key to auto-detect)".into(),
            });
        }
        Ok(())
    }

    /// Read the API key from the env var named by `api_key_env`.
    /// Returns an `InvalidSection` error if it's unset.
    pub fn resolve_api_key(&self) -> Result<String> {
        std::env::var(&self.api_key_env).map_err(|_| Error::InvalidSection {
            section: "stt",
            reason: format!("env var {} is not set", self.api_key_env),
        })
    }
}

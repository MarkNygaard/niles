//! Tier 1 and Tier 2 LLM provider configuration.
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
    /// Optional Tier 2 backend configuration.
    #[serde(default)]
    pub tier2: Option<LlmTier2Config>,
}

/// `[llm.tier2]` section — second LLM tier for escalation.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmTier2Config {
    /// Name of the env var holding the Tier 2 provider API key
    /// (e.g. `"OPENAI_API_KEY"`).
    pub api_key_env: String,
    /// Provider base URL. Defaults to OpenAI's hosted endpoint.
    #[serde(default = "default_tier2_base_url")]
    pub base_url: String,
    /// Model identifier passed to the provider.
    #[serde(default = "default_tier2_model")]
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

fn default_tier2_base_url() -> String {
    "https://api.openai.com/v1".into()
}

fn default_tier2_model() -> String {
    "gpt-5.5".into()
}

fn default_timeout_secs() -> u64 {
    30
}

fn validate_llm_fields(
    section: &'static str,
    api_key_env: &str,
    base_url: &str,
    model: &str,
    timeout_seconds: u64,
) -> Result<()> {
    if api_key_env.trim().is_empty() {
        return Err(Error::InvalidSection {
            section,
            reason: "api_key_env must not be empty".into(),
        });
    }
    if base_url.trim().is_empty() {
        return Err(Error::InvalidSection {
            section,
            reason: "base_url must not be empty".into(),
        });
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(Error::InvalidSection {
            section,
            reason: format!("base_url '{base_url}' must start with http:// or https://"),
        });
    }
    if model.trim().is_empty() {
        return Err(Error::InvalidSection {
            section,
            reason: "model must not be empty".into(),
        });
    }
    if timeout_seconds == 0 {
        return Err(Error::InvalidSection {
            section,
            reason: "timeout_seconds must be > 0".into(),
        });
    }
    Ok(())
}

fn resolve_api_key(api_key_env: &str, section: &'static str) -> Result<String> {
    std::env::var(api_key_env).map_err(|_| Error::InvalidSection {
        section,
        reason: format!("env var {api_key_env} is not set"),
    })
}

impl LlmConfig {
    pub fn validate(&self) -> Result<()> {
        validate_llm_fields(
            "llm",
            &self.api_key_env,
            &self.base_url,
            &self.model,
            self.timeout_seconds,
        )?;
        if let Some(tier2) = &self.tier2 {
            tier2.validate()?;
        }
        Ok(())
    }

    /// Read the API key from the env var named by `api_key_env`.
    /// Returns an `InvalidSection` error if it's unset.
    pub fn resolve_api_key(&self) -> Result<String> {
        resolve_api_key(&self.api_key_env, "llm")
    }
}

impl LlmTier2Config {
    pub fn validate(&self) -> Result<()> {
        validate_llm_fields(
            "llm.tier2",
            &self.api_key_env,
            &self.base_url,
            &self.model,
            self.timeout_seconds,
        )
    }

    /// Read the API key from the env var named by `api_key_env`.
    /// Returns an `InvalidSection` error if it's unset.
    pub fn resolve_api_key(&self) -> Result<String> {
        resolve_api_key(&self.api_key_env, "llm.tier2")
    }
}

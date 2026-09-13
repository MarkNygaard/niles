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
    /// Which `[[providers]]` entry serves this role.
    ///
    /// When set, its endpoint and key are used and the two fields
    /// below are ignored. When absent, they are read as before — which
    /// is what every config written before providers existed does.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
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
    /// Which `[[providers]]` entry serves this role.
    ///
    /// When set, its endpoint and key are used and the two fields
    /// below are ignored. When absent, they are read as before — which
    /// is what every config written before providers existed does.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
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
    // An unnamed key is *unset*, not wrong, and `Config::setup_gaps`
    // reports it. Refusing to load here would mean a Niles nobody has
    // given a key to cannot start far enough to be given one.
    let _ = api_key_env;
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
    // The section doubles as the purpose: `llm` and the tier-2 section
    // are separate keys because they can be separate accounts.
    crate::env::require_secret(section, &format!("{section}.api_key"), api_key_env)
}

/// Every field has a default, so the section can be left out
/// entirely and filled in later from the app.
impl Default for LlmConfig {
    fn default() -> Self {
        toml::from_str("").expect("every field has a default")
    }
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

//! Tier 1 and Tier 2 LLM provider configuration.
//!
//! Same secrets pattern as `[stt]`: the TOML carries the *name* of
//! the env var that holds the API key. The runtime resolves it at
//! startup so secrets stay out of the config file.

use crate::error::{Error, Result};
pub use niles_llm::ReasoningEffort;
use serde::Deserialize;

/// `[llm]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    /// Which `[[providers]]` entry serves this role.
    ///
    /// When set, its endpoint and key are used and `base_url` is
    /// ignored. When absent, the role's own `base_url` is used if it has
    /// one, and otherwise the role's default provider, Cerebras.
    #[serde(default)]
    pub provider: Option<String>,
    /// Name of the env var holding the API key (e.g.
    /// `"CEREBRAS_API_KEY"`), when it comes from the environment rather
    /// than from the app.
    #[serde(default)]
    pub api_key_env: String,
    /// An OpenAI-compatible endpoint of the role's own. Absent means the
    /// provider.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Model identifier passed to the provider.
    #[serde(default = "default_model")]
    pub model: String,
    /// Provider request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_seconds: u64,
    /// How hard the model should think before answering.
    ///
    /// Unset by default, which sends nothing and leaves the provider
    /// to its own — `medium` on the gpt-oss models. `low` is what
    /// Tier 1 usually wants: this tier exists to be quick, and
    /// reasoning tokens are generated while somebody waits for a light
    /// to come on, then charged against the same budget as the answer.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Optional Tier 2 backend configuration.
    #[serde(default)]
    pub tier2: Option<LlmTier2Config>,
}

/// `[llm.tier2]` section — second LLM tier for escalation.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmTier2Config {
    /// Which `[[providers]]` entry serves this role.
    ///
    /// When set, its endpoint and key are used and `base_url` is
    /// ignored. When absent, `base_url` is.
    #[serde(default)]
    pub provider: Option<String>,
    /// Name of the env var holding the Tier 2 API key (e.g.
    /// `"OPENAI_API_KEY"`).
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
    /// How hard the model should think before answering. The tier that
    /// exists because something was hard is the one that can afford it.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
}

// From the catalogue, so the shipped default and the model offered
// in the dropdown are the same string by construction rather than
// because somebody remembered to change both.
fn default_model() -> String {
    crate::catalogue::default_model(crate::providers::Role::Llm).into()
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

// The key is not checked: an unnamed key is *unset*, not wrong, and
// `Config::setup_gaps` reports it. Refusing to load here would mean a
// Niles nobody has given a key to cannot start far enough to be given
// one.
fn validate_llm_fields(
    section: &'static str,
    base_url: Option<&str>,
    model: &str,
    timeout_seconds: u64,
) -> Result<()> {
    if let Some(base_url) = base_url {
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
    /// The provider this role is served by: the one it names, or its
    /// default when it names none and has no endpoint of its own.
    pub fn provider_in_use(&self) -> Option<&str> {
        match (&self.provider, &self.base_url) {
            (Some(name), _) => Some(name),
            (None, Some(_)) => None,
            (None, None) => Some(crate::catalogue::default_provider(crate::Role::Llm).id),
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_llm_fields(
            "llm",
            self.base_url.as_deref(),
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
            Some(&self.base_url),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn llm(toml: &str) -> LlmConfig {
        toml::from_str(toml).expect("valid")
    }

    #[test]
    fn nothing_is_asked_for_unless_somebody_asks() {
        // Unset is the default on purpose: not every model accepts the
        // field, and a value nobody chose is a 400 from somebody
        // else's server.
        assert_eq!(LlmConfig::default().reasoning_effort, None);
    }

    #[test]
    fn the_effort_is_read_off_the_config() {
        assert_eq!(
            llm("reasoning_effort = \"low\"").reasoning_effort,
            Some(ReasoningEffort::Low)
        );
        assert_eq!(
            llm("reasoning_effort = \"high\"").reasoning_effort,
            Some(ReasoningEffort::High)
        );
    }

    #[test]
    fn a_word_no_provider_knows_is_refused_here() {
        // At load, where it names the file, rather than as a 400 in the
        // middle of somebody asking for the lights.
        assert!(toml::from_str::<LlmConfig>("reasoning_effort = \"maximum\"").is_err());
    }

    #[test]
    fn the_escalation_tier_has_its_own() {
        // The two tiers want opposite things: Tier 1 is quick, Tier 2
        // exists because something was hard.
        let cfg = llm(r#"
            reasoning_effort = "low"
            [tier2]
            reasoning_effort = "high"
            "#);
        assert_eq!(cfg.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(
            cfg.tier2.expect("present").reasoning_effort,
            Some(ReasoningEffort::High)
        );
    }
}

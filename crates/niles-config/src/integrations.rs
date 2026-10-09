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

fn default_true() -> bool {
    true
}

fn default_claude_model() -> String {
    "sonnet".into()
}

fn default_claude_timeout_seconds() -> u64 {
    120
}

/// Top-level `[integrations]` section of the config file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub linear: Option<LinearConfigDto>,
    #[serde(default)]
    pub claude_code: Option<ClaudeCodeConfig>,
    #[serde(default)]
    pub nemlig: Option<NemligConfig>,
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
}

/// `[integrations.spotify]` — a Spotify developer app, for finding what
/// to play. Sonos plays it, through the Spotify account linked in the
/// Sonos app; this only searches the catalogue, which needs no login.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpotifyConfig {
    /// Off keeps the section and the stored keys without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// The variables holding the app's keys, when they come from them
    /// rather than from Niles's own store.
    #[serde(default)]
    pub client_id_env: String,
    #[serde(default)]
    pub client_secret_env: String,
}

impl SpotifyConfig {
    /// The app's keys: (client id, client secret).
    pub fn resolve_credentials(&self) -> Result<(String, String)> {
        let id = crate::env::require_secret(
            "integrations.spotify",
            "integrations.spotify.client_id",
            &self.client_id_env,
        )?;
        let secret = crate::env::require_secret(
            "integrations.spotify",
            "integrations.spotify.client_secret",
            &self.client_secret_env,
        )?;
        Ok((id, secret))
    }
}

/// `[integrations.nemlig]` — the household's nemlig.com account, for
/// picking the product behind each item on the shopping list and
/// filling the basket. Paying stays at nemlig.com.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NemligConfig {
    /// Off keeps the section and the stored login without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// The variables holding the login, when it comes from them rather
    /// than from Niles's own store.
    #[serde(default)]
    pub username_env: String,
    #[serde(default)]
    pub password_env: String,
}

impl NemligConfig {
    /// The login: (email, password).
    pub fn resolve_credentials(&self) -> Result<(String, String)> {
        let username = crate::env::require_secret(
            "integrations.nemlig",
            "integrations.nemlig.username",
            &self.username_env,
        )?;
        let password = crate::env::require_secret(
            "integrations.nemlig",
            "integrations.nemlig.password",
            &self.password_env,
        )?;
        Ok((username, password))
    }
}

/// `[integrations.claude_code]` — the app's chat answered by Claude Code,
/// on the household's Claude subscription.
///
/// Only the chat: a spoken command still goes to `[llm]`, because a
/// process that takes seconds to start has no place in a voice reply.
/// When this is off, or Claude Code fails, the chat falls back to
/// `[llm]` as well.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeCodeConfig {
    /// Off keeps the section (and the model chosen) without using it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `sonnet`, `opus`, `haiku`, or a full model id.
    #[serde(default = "default_claude_model")]
    pub model: String,
    /// The variable holding the token from `claude setup-token`, when it
    /// comes from one rather than from Niles's own store.
    #[serde(default)]
    pub oauth_token_env: String,
    /// How long one answer may take, tool calls included.
    #[serde(default = "default_claude_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl ClaudeCodeConfig {
    pub fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.claude_code",
                reason: "model must not be empty".into(),
            });
        }
        if self.timeout_seconds < 10 || self.timeout_seconds > 600 {
            return Err(Error::InvalidSection {
                section: "integrations.claude_code",
                reason: "timeout_seconds must be between 10 and 600".into(),
            });
        }
        Ok(())
    }

    /// The token from `claude setup-token`.
    pub fn resolve_oauth_token(&self) -> Result<String> {
        crate::env::require_secret(
            "integrations.claude_code",
            "integrations.claude_code.oauth_token",
            &self.oauth_token_env,
        )
    }
}

/// `[integrations.linear]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearConfigDto {
    /// The variable holding the key, when it comes from one.
    ///
    /// Optional now that a credential can be stored instead: naming no
    /// variable means Niles looks in its own store, and says so clearly
    /// if it is not there either. Requiring a name here made the
    /// Settings page unable to set up Linear at all.
    #[serde(default)]
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
        if let Some(claude) = &self.claude_code {
            claude.validate()?;
        }
        Ok(())
    }
}

impl LinearConfigDto {
    pub fn validate(&self) -> Result<()> {
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
        crate::env::require_secret(
            "integrations.linear",
            "integrations.linear.api_key",
            &self.api_key_env,
        )
    }
}

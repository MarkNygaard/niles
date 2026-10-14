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

fn default_assistant_timeout_seconds() -> u64 {
    120
}

/// Balanced, as Sonnet is for Claude: the everyday GPT-5.6 tier, quick
/// enough for a chat and careful enough to build a rule.
fn default_chatgpt_model() -> String {
    "gpt-5.6-terra".into()
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
    pub chatgpt: Option<ChatGptConfig>,
    /// Which assistant answers when both are added, switched on and
    /// signed in. With only one of them usable, that one answers
    /// whatever this says.
    #[serde(default)]
    pub assistant: Assistant,
    #[serde(default)]
    pub nemlig: Option<NemligConfig>,
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
    #[serde(default)]
    pub microsoft: Option<MicrosoftConfig>,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
}

/// `[integrations.google]` — the household's OAuth client in Google
/// Cloud, through which each person connects their own Google calendar.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleConfig {
    /// Off keeps the section, the stored keys and every person's
    /// connections without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub client_id_env: String,
    #[serde(default)]
    pub client_secret_env: String,
    /// The redirect URI registered on the client; compared by Google as
    /// Microsoft compares its own. Left out, it is built from the
    /// address the browser used.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

impl GoogleConfig {
    /// The client's keys: (client id, client secret).
    pub fn resolve_credentials(&self) -> Result<(String, String)> {
        let id = crate::env::require_secret(
            "integrations.google",
            "integrations.google.client_id",
            &self.client_id_env,
        )?;
        let secret = crate::env::require_secret(
            "integrations.google",
            "integrations.google.client_secret",
            &self.client_secret_env,
        )?;
        Ok((id, secret))
    }
}

/// `[integrations.microsoft]` — the household's app registration in
/// Microsoft Entra, through which each person connects their own
/// calendars: an Outlook.com account and a work one alike.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MicrosoftConfig {
    /// Off keeps the section, the stored keys and every person's
    /// connections without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub client_id_env: String,
    #[serde(default)]
    pub client_secret_env: String,
    /// The redirect URI registered on the app. Microsoft compares it
    /// letter for letter, at sign-in and again at the token exchange;
    /// left out, it is built from the address the browser used.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

impl MicrosoftConfig {
    /// The app's keys: (client id, client secret).
    pub fn resolve_credentials(&self) -> Result<(String, String)> {
        let id = crate::env::require_secret(
            "integrations.microsoft",
            "integrations.microsoft.client_id",
            &self.client_id_env,
        )?;
        let secret = crate::env::require_secret(
            "integrations.microsoft",
            "integrations.microsoft.client_secret",
            &self.client_secret_env,
        )?;
        Ok((id, secret))
    }
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

/// The assistants that can answer in Niles's place, each on a
/// household's own subscription.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assistant {
    #[default]
    ClaudeCode,
    Chatgpt,
}

/// `[integrations.claude_code]` — Claude Code, on the household's Claude
/// subscription, as the assistant.
///
/// It answers the app's chat, a spoken turn about the house's rules (the
/// satellite is kept waiting while it does), and the review of what was
/// said. Other spoken commands still go to `[llm]`: a process that takes
/// seconds to start has no place in "lights off". When this is off, or
/// Claude Code fails, everything falls back to `[llm]` as well.
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
    #[serde(default = "default_assistant_timeout_seconds")]
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

/// `[integrations.chatgpt]` — the same assistant as Claude Code, on a
/// household's ChatGPT subscription instead, run through the pi agent.
///
/// The sign-in is not here: it is made in the app and kept in the
/// secret store, under the keys [`ChatGptSignIn`] names.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatGptConfig {
    /// Off keeps the section, the model and the sign-in without using
    /// them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// A model the subscription offers through Codex: `gpt-5.6-terra`,
    /// `gpt-6-astra`, …
    #[serde(default = "default_chatgpt_model")]
    pub model: String,
    /// How long one answer may take, tool calls included.
    #[serde(default = "default_assistant_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl ChatGptConfig {
    pub fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "integrations.chatgpt",
                reason: "model must not be empty".into(),
            });
        }
        if self.timeout_seconds < 10 || self.timeout_seconds > 600 {
            return Err(Error::InvalidSection {
                section: "integrations.chatgpt",
                reason: "timeout_seconds must be between 10 and 600".into(),
            });
        }
        Ok(())
    }
}

/// A ChatGPT sign-in, as the secret store keeps it.
///
/// Four keys rather than one blob so each is an ordinary stored secret:
/// removing the integration deletes them like any other, and the page
/// can tell whether one is there without reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptSignIn {
    pub access_token: String,
    /// OpenAI rotates it on every use, so Niles alone may spend it.
    pub refresh_token: String,
    pub account_id: String,
    /// When the access token stops working, in seconds since the epoch.
    pub expires_at: i64,
}

impl ChatGptSignIn {
    pub const ACCESS_TOKEN: &str = "integrations.chatgpt.access_token";
    pub const REFRESH_TOKEN: &str = "integrations.chatgpt.refresh_token";
    pub const ACCOUNT_ID: &str = "integrations.chatgpt.account_id";
    pub const EXPIRES_AT: &str = "integrations.chatgpt.expires_at";
    pub const KEYS: [&str; 4] = [
        Self::ACCESS_TOKEN,
        Self::REFRESH_TOKEN,
        Self::ACCOUNT_ID,
        Self::EXPIRES_AT,
    ];

    /// The sign-in in the secret store, if there is a whole one.
    pub fn stored() -> Option<Self> {
        Some(Self {
            access_token: crate::secrets::get(Self::ACCESS_TOKEN)?,
            refresh_token: crate::secrets::get(Self::REFRESH_TOKEN)?,
            account_id: crate::secrets::get(Self::ACCOUNT_ID)?,
            // Unreadable reads as expired, which costs one refresh.
            expires_at: crate::secrets::get(Self::EXPIRES_AT)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
        })
    }

    /// What to save, refresh token first: if saving stops part way, a
    /// rotated refresh token is the one thing that cannot be got back.
    pub fn entries(&self) -> [(&'static str, String); 4] {
        [
            (Self::REFRESH_TOKEN, self.refresh_token.clone()),
            (Self::ACCESS_TOKEN, self.access_token.clone()),
            (Self::ACCOUNT_ID, self.account_id.clone()),
            (Self::EXPIRES_AT, self.expires_at.to_string()),
        ]
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
        if let Some(chatgpt) = &self.chatgpt {
            chatgpt.validate()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn integrations(toml: &str) -> IntegrationsConfig {
        toml::from_str(toml).expect("valid")
    }

    #[test]
    fn claude_answers_unless_told_otherwise() {
        assert_eq!(integrations("").assistant, Assistant::ClaudeCode);
        assert_eq!(
            integrations("assistant = \"chatgpt\"").assistant,
            Assistant::Chatgpt
        );
        assert!(toml::from_str::<IntegrationsConfig>("assistant = \"gemini\"").is_err());
    }

    #[test]
    fn an_empty_chatgpt_section_is_a_working_one() {
        let chatgpt = integrations("[chatgpt]").chatgpt.expect("added");
        assert!(chatgpt.enabled);
        assert_eq!(chatgpt.model, "gpt-5.6-terra");
        assert_eq!(chatgpt.timeout_seconds, 120);
        assert!(chatgpt.validate().is_ok());
    }

    #[test]
    fn chatgpt_is_held_to_the_same_limits_as_claude() {
        assert!(
            integrations("[chatgpt]\ntimeout_seconds = 5")
                .validate()
                .is_err()
        );
        assert!(integrations("[chatgpt]\nmodel = \" \"").validate().is_err());
        // The sign-in is not config, and a key for it here is a mistake.
        assert!(toml::from_str::<IntegrationsConfig>("[chatgpt]\naccess_token = \"x\"").is_err());
    }

    #[test]
    fn a_sign_in_is_read_back_whole_or_not_at_all() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        let sign_in = ChatGptSignIn {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            account_id: "acct".into(),
            expires_at: 1_900_000_000,
        };
        let stored = |skip: Option<&str>| {
            sign_in
                .entries()
                .into_iter()
                .filter(|(k, _)| Some(*k) != skip)
                .map(|(k, v)| (k.to_string(), v))
                .collect()
        };
        crate::secrets::load(stored(None));
        assert_eq!(ChatGptSignIn::stored(), Some(sign_in.clone()));
        crate::secrets::load(stored(Some(ChatGptSignIn::REFRESH_TOKEN)));
        assert_eq!(ChatGptSignIn::stored(), None);
        crate::secrets::load(Default::default());
    }

    #[test]
    fn the_refresh_token_is_saved_first() {
        let sign_in = ChatGptSignIn {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            account_id: "acct".into(),
            expires_at: 0,
        };
        assert_eq!(
            sign_in.entries()[0],
            (ChatGptSignIn::REFRESH_TOKEN, "rt".to_string())
        );
        let mut keys: Vec<&str> = sign_in.entries().iter().map(|(k, _)| *k).collect();
        keys.sort();
        let mut known = ChatGptSignIn::KEYS.to_vec();
        known.sort();
        assert_eq!(keys, known);
    }
}

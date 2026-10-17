//! Integrations configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

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
    /// Retired: Linear is a [tool server](crate::tool_servers) now.
    /// Still read so a stored override that has it loads rather than
    /// being refused whole, then dropped as the config settles.
    #[serde(default)]
    pub linear: Option<toml::Table>,
    #[serde(default)]
    pub claude_code: Option<ClaudeCodeConfig>,
    #[serde(default)]
    pub chatgpt: Option<ChatGptConfig>,
    /// Which assistant answers when both are added, switched on and
    /// signed in. With only one of them usable, that one answers
    /// whatever this says — unless it says `none`, which keeps both out.
    #[serde(default)]
    pub assistant: Assistant,
    #[serde(default)]
    pub nemlig: Option<NemligConfig>,
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
    /// Set up in the app (Settings → Add-ons → Podcasts), never written
    /// into a config file by hand.
    #[serde(default)]
    pub podcasts: Option<PodcastsConfig>,
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

/// `[integrations.podcasts]` — the household's own Kokoro (which speaks
/// the episodes) and a private Cloudflare R2 bucket (which keeps them),
/// for podcasts researched by the assistant and recorded by Niles (see
/// ARCHITECTURE.md § Podcasts).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PodcastsConfig {
    /// Where Kokoro-FastAPI runs, such as `http://kokoro:8880`. A
    /// trailing `/v1` or `/` is no problem.
    #[serde(default)]
    pub kokoro_url: String,
    /// The account's S3 endpoint, such as
    /// `https://<account-id>.r2.cloudflarestorage.com`.
    #[serde(default)]
    pub endpoint: String,
    /// The bucket episodes are kept in.
    #[serde(default)]
    pub bucket: String,
    /// The variables holding the R2 API token's two parts, when they
    /// come from them rather than from Niles's own store.
    #[serde(default)]
    pub access_key_id_env: String,
    #[serde(default)]
    pub secret_access_key_env: String,
    /// The SigV4 region used for signing and presigning.
    #[serde(default = "default_podcasts_region")]
    pub region: String,
    /// The two hosts' Kokoro voices, English ones.
    #[serde(default = "default_first_voice")]
    pub first_voice: String,
    #[serde(default = "default_second_voice")]
    pub second_voice: String,
    /// How many days an episode is kept.
    #[serde(default = "default_keep_days")]
    pub keep_days: u16,
}

pub const DEFAULT_REGION: &str = "auto";
pub const DEFAULT_VOICES: [&str; 2] = ["af_heart", "am_michael"];
pub const DEFAULT_KEEP_DAYS: u16 = 14;

fn default_podcasts_region() -> String {
    DEFAULT_REGION.into()
}

fn default_first_voice() -> String {
    DEFAULT_VOICES[0].into()
}

fn default_second_voice() -> String {
    DEFAULT_VOICES[1].into()
}

fn default_keep_days() -> u16 {
    DEFAULT_KEEP_DAYS
}

impl Default for PodcastsConfig {
    fn default() -> Self {
        Self {
            kokoro_url: String::new(),
            endpoint: String::new(),
            bucket: String::new(),
            access_key_id_env: String::new(),
            secret_access_key_env: String::new(),
            region: default_podcasts_region(),
            first_voice: default_first_voice(),
            second_voice: default_second_voice(),
            keep_days: default_keep_days(),
        }
    }
}

impl PodcastsConfig {
    /// Kokoro's address, without a trailing `/v1/` or `/`; nothing
    /// when it is not set.
    pub fn kokoro_address(&self) -> Option<String> {
        let mut address = self.kokoro_url.trim().to_string();
        while address.ends_with('/') {
            address.pop();
        }
        if let Some(stripped) = address.strip_suffix("/v1") {
            address = stripped.to_string();
        }
        while address.ends_with('/') {
            address.pop();
        }
        (!address.is_empty()).then_some(address)
    }

    /// The bucket endpoint's address, without a trailing `/`; nothing
    /// when it is not set.
    pub fn endpoint_address(&self) -> Option<String> {
        let mut address = self.endpoint.trim().to_string();
        while address.ends_with('/') {
            address.pop();
        }
        (!address.is_empty()).then_some(address)
    }

    /// The bucket's name; nothing when it is not set.
    pub fn bucket_name(&self) -> Option<&str> {
        let name = self.bucket.trim();
        (!name.is_empty()).then_some(name)
    }

    /// The SigV4 region: `auto` for R2, whatever else an S3-compatible
    /// service wants.
    pub fn signing_region(&self) -> &str {
        let region = self.region.trim();
        if region.is_empty() {
            DEFAULT_REGION
        } else {
            region
        }
    }

    /// The two hosts' voices, each falling back to its default when
    /// left blank.
    pub fn voices(&self) -> [String; 2] {
        [
            voice_or_default(&self.first_voice, DEFAULT_VOICES[0]),
            voice_or_default(&self.second_voice, DEFAULT_VOICES[1]),
        ]
    }

    /// The R2 API token's access key id.
    pub fn access_key_id(&self) -> Result<String> {
        Ok(crate::env::require_secret(
            "integrations.podcasts",
            "integrations.podcasts.access_key_id",
            &self.access_key_id_env,
        )?
        .trim()
        .to_string())
    }

    /// The R2 API token's secret access key.
    pub fn secret_access_key(&self) -> Result<String> {
        Ok(crate::env::require_secret(
            "integrations.podcasts",
            "integrations.podcasts.secret_access_key",
            &self.secret_access_key_env,
        )?
        .trim()
        .to_string())
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |reason: String| Error::InvalidSection {
            section: "integrations.podcasts",
            reason,
        };
        let kokoro = self.kokoro_url.trim();
        if !(kokoro.is_empty() || kokoro.starts_with("http://") || kokoro.starts_with("https://")) {
            return Err(invalid(format!(
                "kokoro_url {kokoro:?} must start with http:// or https://"
            )));
        }
        let endpoint = self.endpoint.trim();
        if !endpoint.is_empty() {
            let rest = endpoint
                .strip_prefix("https://")
                .or_else(|| endpoint.strip_prefix("http://"));
            let host_ok = match rest {
                Some(host) => {
                    let host = host.strip_suffix('/').unwrap_or(host);
                    !host.is_empty()
                        && !host.contains('/')
                        && !host.contains(['?', '#'])
                        && !host.chars().any(char::is_whitespace)
                }
                None => false,
            };
            if !host_ok {
                return Err(invalid(
                    "endpoint is the account's S3 address, such as \
                     https://<account-id>.r2.cloudflarestorage.com, with nothing after it (the \
                     bucket has its own field)"
                        .into(),
                ));
            }
        }
        let bucket = self.bucket.trim();
        if !bucket.is_empty() {
            let name_char =
                |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-';
            let well_formed = bucket.len() >= 3
                && bucket.len() <= 63
                && bucket.chars().all(name_char)
                && bucket.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
                && bucket.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
            if !well_formed {
                return Err(invalid(format!(
                    "bucket {bucket:?} is not a bucket name: 3 to 63 lowercase letters, digits, \
                     dots and hyphens"
                )));
            }
        }
        let region = self.region.trim();
        if !region.is_empty()
            && (region.len() > 32
                || !region
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        {
            return Err(invalid(format!(
                "region {region:?} must be lowercase letters, digits and hyphens, such as auto"
            )));
        }
        let voices = self.voices();
        for voice in &voices {
            let english = voice.len() >= 4
                && matches!(voice.as_bytes()[0], b'a' | b'b')
                && voice.as_bytes()[1].is_ascii_lowercase()
                && voice.as_bytes()[2] == b'_'
                && voice[3..]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !english {
                return Err(invalid(format!(
                    "Podcasts are in English for now: {voice:?} is not an English Kokoro voice \
                     (one starting af_, am_, bf_ or bm_)"
                )));
            }
        }
        if voices[0] == voices[1] {
            return Err(invalid("the two hosts need different voices".into()));
        }
        if !(1..=365).contains(&self.keep_days) {
            return Err(invalid("keep_days must be between 1 and 365".into()));
        }
        Ok(())
    }
}

fn voice_or_default(raw: &str, default: &str) -> String {
    let voice = raw.trim();
    if voice.is_empty() {
        default.to_string()
    } else {
        voice.to_string()
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
    /// Neither answers, even when one is set up: Niles's own model
    /// handles the chat and rules, and nothing runs in the background.
    /// For a house that keeps a subscription for other things and does
    /// not want it spent here.
    None,
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

impl IntegrationsConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(claude) = &self.claude_code {
            claude.validate()?;
        }
        if let Some(chatgpt) = &self.chatgpt {
            chatgpt.validate()?;
        }
        if let Some(podcasts) = &self.podcasts {
            podcasts.validate()?;
        }
        Ok(())
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
        assert_eq!(
            integrations("assistant = \"none\"").assistant,
            Assistant::None
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

    fn podcasts(body: &str) -> PodcastsConfig {
        let toml = if body.trim().is_empty() {
            "[podcasts]".to_string()
        } else {
            format!("[podcasts]\n{body}")
        };
        toml::from_str::<IntegrationsConfig>(&toml)
            .expect("valid")
            .podcasts
            .expect("the section")
    }

    #[test]
    fn an_empty_podcasts_section_is_a_half_set_up_one_that_saves() {
        let podcasts = podcasts("");
        assert_eq!(podcasts.signing_region(), "auto");
        assert_eq!(podcasts.voices(), ["af_heart", "am_michael"]);
        assert_eq!(podcasts.keep_days, 14);
        assert!(podcasts.validate().is_ok());
        assert!(toml::from_str::<IntegrationsConfig>("[podcasts]\naccess_key_id = \"x\"").is_err());
    }

    #[test]
    fn kokoro_address_strips_a_trailing_v1() {
        let mut podcasts = podcasts("");
        podcasts.kokoro_url = " http://kokoro:8880/v1/ ".into();
        assert_eq!(
            podcasts.kokoro_address().as_deref(),
            Some("http://kokoro:8880")
        );
        podcasts.kokoro_url = "https://kokoro.example.com".into();
        assert_eq!(
            podcasts.kokoro_address().as_deref(),
            Some("https://kokoro.example.com")
        );
        podcasts.kokoro_url = " ".into();
        assert_eq!(podcasts.kokoro_address(), None);
    }

    #[test]
    fn an_endpoint_with_a_bucket_path_is_refused() {
        let mut podcasts = podcasts("");
        podcasts.endpoint = "https://account.r2.cloudflarestorage.com/my-bucket".into();
        let error = podcasts.validate().expect_err("refused");
        let reason = error.to_string();
        assert!(
            reason.contains("with nothing after it"),
            "unexpected: {reason}"
        );
        podcasts.endpoint = "https://account.r2.cloudflarestorage.com/".into();
        assert!(podcasts.validate().is_ok());
        podcasts.endpoint = "account.r2.cloudflarestorage.com".into();
        assert!(podcasts.validate().is_err());
    }

    #[test]
    fn a_bucket_name_is_lowercase_and_bounded() {
        let mut podcasts = podcasts("");
        podcasts.bucket = "Niles-Podcasts".into();
        let error = podcasts.validate().expect_err("refused");
        assert!(
            error.to_string().contains("is not a bucket name"),
            "unexpected: {error}"
        );
        podcasts.bucket = "niles-podcasts".into();
        assert!(podcasts.validate().is_ok());
        podcasts.bucket = "ab".into();
        assert!(podcasts.validate().is_err());
    }

    #[test]
    fn the_two_hosts_need_different_english_voices() {
        let mut podcasts = podcasts("");
        podcasts.first_voice = "af_heart".into();
        podcasts.second_voice = "af_heart".into();
        let error = podcasts.validate().expect_err("refused");
        assert!(
            error.to_string().contains("different voices"),
            "unexpected: {error}"
        );
        podcasts.second_voice = "ef_dora".into();
        let error = podcasts.validate().expect_err("refused");
        assert!(
            error.to_string().contains("not an English Kokoro voice"),
            "unexpected: {error}"
        );
        podcasts.second_voice = "bm_george".into();
        assert!(podcasts.validate().is_ok());
    }

    #[test]
    fn keep_days_is_a_year_at_most() {
        let mut podcasts = podcasts("");
        podcasts.keep_days = 0;
        assert!(podcasts.validate().is_err());
        podcasts.keep_days = 366;
        assert!(podcasts.validate().is_err());
        podcasts.keep_days = 365;
        assert!(podcasts.validate().is_ok());
    }

    #[test]
    fn a_blank_region_signs_as_auto() {
        let mut podcasts = podcasts("");
        podcasts.region = " ".into();
        assert_eq!(podcasts.signing_region(), "auto");
        podcasts.region = "eu-west-1".into();
        assert_eq!(podcasts.signing_region(), "eu-west-1");
        podcasts.region = "EU_WEST".into();
        assert!(podcasts.validate().is_err());
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

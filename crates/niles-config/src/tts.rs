//! Text-to-speech provider configuration.
//!
//! Piper is self-hosted in-cluster, so no API key — just URL + default
//! voice + timeout. Same `http(s)://` prefix check as `[stt]`.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[tts]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsConfig {
    /// Piper HTTP endpoint. Defaults to the in-cluster service DNS.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Default voice model (e.g. `"en_GB-alan-medium"`).
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Provider request timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_base_url() -> String {
    "http://piper.home-automation.svc.cluster.local:5000".into()
}

fn default_voice() -> String {
    "en_GB-alan-medium".into()
}

fn default_timeout_seconds() -> u64 {
    30
}

impl TtsConfig {
    pub fn validate(&self) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "tts",
                reason: "base_url must not be empty".into(),
            });
        }
        // Fail fast on the obvious typo (`htps://...`) at startup
        // rather than on first synthesis. Skips a full URL-parse
        // dep — reqwest catches anything subtler.
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(Error::InvalidSection {
                section: "tts",
                reason: format!(
                    "base_url '{}' must start with http:// or https://",
                    self.base_url
                ),
            });
        }
        if self.default_voice.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "tts",
                reason: "default_voice must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 {
            return Err(Error::InvalidSection {
                section: "tts",
                reason: "timeout_seconds must be > 0".into(),
            });
        }
        Ok(())
    }
}

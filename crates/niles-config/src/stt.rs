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
    /// Optional ISO-639-1 language hint (e.g. `"en"`). `None` lets
    /// Whisper auto-detect.
    #[serde(default)]
    pub language: Option<String>,
    /// Provider request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_seconds: u64,
    /// When to disbelieve a transcript outright.
    #[serde(default)]
    pub noise_gate: NoiseGate,
}

/// When Whisper's own numbers say that was not speech.
///
/// A satellite is woken by a door closing and Whisper is asked to
/// transcribe a room with nothing said in it. It does not return
/// nothing — it returns a short, ordinary, invented sentence, because
/// that is what a model trained to produce text does with noise. No
/// rule about sentence shape separates an invented "thank you" from a
/// real one. These two numbers do.
///
/// **Both** have to be damning, which is the whole design. Either one
/// alone fires on real speech often enough to be worse than the
/// problem: a quiet command scores badly on `no_speech_prob`, and an
/// unusual one scores badly on `avg_logprob`. Together they mean
/// Whisper both heard no speech *and* was guessing at the words.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseGate {
    /// Off by default. The numbers are logged either way, so a house
    /// can be looked at before it is filtered.
    #[serde(default)]
    pub enabled: bool,
    /// Drop above this. 0.6 is OpenAI's own suggestion.
    #[serde(default = "default_no_speech_prob")]
    pub no_speech_prob: f64,
    /// And below this. Less than -1 is a model guessing.
    #[serde(default = "default_avg_logprob")]
    pub avg_logprob: f64,
}

fn default_no_speech_prob() -> f64 {
    0.6
}

fn default_avg_logprob() -> f64 {
    -1.0
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self {
            enabled: false,
            no_speech_prob: default_no_speech_prob(),
            avg_logprob: default_avg_logprob(),
        }
    }
}

impl NoiseGate {
    /// Whether to disbelieve a transcript with these scores.
    pub fn rejects(&self, c: niles_stt::Confidence) -> bool {
        self.enabled && c.no_speech_prob > self.no_speech_prob && c.avg_logprob < self.avg_logprob
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.no_speech_prob) {
            return Err(Error::InvalidSection {
                section: "stt.noise_gate",
                reason: "no_speech_prob is a probability, so it must be in 0..=1".into(),
            });
        }
        // A logprob is never positive, and a threshold of 0 would
        // reject every transcript the other half also flagged.
        if self.avg_logprob >= 0.0 {
            return Err(Error::InvalidSection {
                section: "stt.noise_gate",
                reason: "avg_logprob must be negative".into(),
            });
        }
        Ok(())
    }
}

fn default_base_url() -> String {
    crate::catalogue::default_base_url().into()
}

// From the catalogue, so the shipped default and the model offered
// in the dropdown are the same string by construction rather than
// because somebody remembered to change both.
fn default_model() -> String {
    crate::catalogue::default_model(crate::providers::Role::Stt).into()
}

fn default_timeout_secs() -> u64 {
    30
}

/// Every field has a default, so the section can be left out
/// entirely and filled in later from the app.
impl Default for SttConfig {
    fn default() -> Self {
        toml::from_str("").expect("every field has a default")
    }
}

impl SttConfig {
    pub fn validate(&self) -> Result<()> {
        if self.base_url.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: "base_url must not be empty".into(),
            });
        }
        // Fail fast on the obvious typo (`htps://...`) at startup
        // rather than on first transcription. Skips a full URL-parse
        // dep — reqwest catches anything subtler.
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(Error::InvalidSection {
                section: "stt",
                reason: format!(
                    "base_url '{}' must start with http:// or https://",
                    self.base_url
                ),
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
        self.noise_gate.validate()?;
        Ok(())
    }

    /// Read the API key from the env var named by `api_key_env`.
    /// Returns an `InvalidSection` error if it's unset.
    pub fn resolve_api_key(&self) -> Result<String> {
        crate::env::require_secret("stt", "stt.api_key", &self.api_key_env)
    }
}

#[cfg(test)]
mod noise_gate_tests {
    use super::*;

    fn heard(no_speech_prob: f64, avg_logprob: f64) -> niles_stt::Confidence {
        niles_stt::Confidence {
            no_speech_prob,
            avg_logprob,
        }
    }

    /// A door closing: Whisper is sure there was no speech, and was
    /// guessing at the words it produced anyway.
    const DOOR: (f64, f64) = (0.92, -1.6);

    fn on() -> NoiseGate {
        NoiseGate {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_is_filtered_until_somebody_turns_it_on() {
        // The numbers are logged either way, so a house can be looked
        // at before it is filtered. Taking a threshold off a blog post
        // and applying it to somebody's kitchen is how real commands
        // start disappearing.
        let gate = NoiseGate::default();
        assert!(!gate.enabled);
        assert!(!gate.rejects(heard(DOOR.0, DOOR.1)));
    }

    #[test]
    fn a_door_closing_is_not_a_command() {
        assert!(on().rejects(heard(DOOR.0, DOOR.1)));
    }

    #[test]
    fn one_bad_number_is_not_enough() {
        // This is the whole design. A quiet command scores badly on
        // no_speech_prob; an unusual one scores badly on avg_logprob.
        // Acting on either alone drops real speech.
        assert!(!on().rejects(heard(0.95, -0.2)), "quiet but confident");
        assert!(
            !on().rejects(heard(0.05, -1.9)),
            "unusual, but clearly speech"
        );
    }

    #[test]
    fn an_ordinary_command_is_left_alone() {
        assert!(!on().rejects(heard(0.01, -0.25)));
    }

    #[test]
    fn the_thresholds_can_be_moved() {
        // Which is what the logging is for: a room is tuned from its
        // own numbers, not from the shipped ones.
        let strict = NoiseGate {
            enabled: true,
            no_speech_prob: 0.3,
            avg_logprob: -0.5,
        };
        assert!(strict.rejects(heard(0.4, -0.7)));
        assert!(!on().rejects(heard(0.4, -0.7)));
    }

    #[test]
    fn a_number_outside_its_range_is_refused_at_load() {
        // At load, where it names the section, rather than as silence
        // in the middle of somebody asking for the lights.
        assert!(
            NoiseGate {
                no_speech_prob: 1.4,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            NoiseGate {
                avg_logprob: 0.5,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(NoiseGate::default().validate().is_ok());
    }

    #[test]
    fn the_section_can_be_left_out_entirely() {
        let cfg: SttConfig = toml::from_str("").expect("valid");
        assert!(!cfg.noise_gate.enabled);
        assert_eq!(cfg.noise_gate.no_speech_prob, 0.6);
        assert_eq!(cfg.noise_gate.avg_logprob, -1.0);
    }

    #[test]
    fn it_is_read_off_the_config() {
        let cfg: SttConfig = toml::from_str(
            r#"
            [noise_gate]
            enabled = true
            no_speech_prob = 0.75
            "#,
        )
        .expect("valid");
        assert!(cfg.noise_gate.enabled);
        assert_eq!(cfg.noise_gate.no_speech_prob, 0.75);
        // Untouched keys keep their shipped value.
        assert_eq!(cfg.noise_gate.avg_logprob, -1.0);
        assert!(cfg.validate().is_ok());
    }
}

//! Groq Whisper transcription client.
//!
//! Targets the OpenAI-compatible `POST /openai/v1/audio/transcriptions`
//! endpoint. Audio is uploaded as multipart/form-data; the JSON
//! response carries the transcript and (optionally) the detected
//! language and audio duration.
//!
//! Groq's hosted Whisper is request-response, not streaming — for a
//! Wyoming voice loop we accumulate audio chunks between
//! `audio-start` and `audio-stop` and submit the whole buffer once.
//! The model itself runs at ~200ms for short utterances so this is
//! within the latency budget.

use crate::error::{Error, Result};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use std::time::Duration;
use tracing::debug;

/// Defaults pointing at Groq's hosted Whisper. Override per
/// deployment via [`WhisperConfig`].
const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Inputs to [`WhisperClient::new`]. Keeps configuration explicit so
/// the binary's config-loading layer is the only place that reads
/// env vars or files.
#[derive(Debug, Clone)]
pub struct WhisperConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// ISO-639-1 hint, e.g. `Some("en")`. None = auto-detect.
    pub language: Option<String>,
    pub request_timeout: Duration,
}

impl WhisperConfig {
    /// Construct with sensible defaults: Groq's public endpoint, the
    /// recommended turbo model, no language hint, 30s timeout.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.into(),
            model: DEFAULT_MODEL.into(),
            language: None,
            request_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }
}

/// Successful transcription. Fields beyond `text` are best-effort:
/// Groq returns them today via `verbose_json` but a future provider
/// might not.
#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub duration_seconds: Option<f64>,
}

/// HTTP client around Groq's Whisper endpoint. Owns its own
/// `reqwest::Client` so caller wiring stays a one-liner.
pub struct WhisperClient {
    http: reqwest::Client,
    cfg: WhisperConfig,
}

impl WhisperClient {
    pub fn new(cfg: WhisperConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        Ok(Self { http, cfg })
    }

    /// Submit audio bytes (any format Groq accepts — WAV, MP3, FLAC,
    /// OGG, M4A, etc.) and return the transcript. `filename` is sent
    /// to the server purely so the multipart parser can sniff the
    /// type from the extension; the content is what actually matters.
    pub async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<Transcript> {
        let url = format!(
            "{}/audio/transcriptions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let part = Part::bytes(audio).file_name(filename.to_string());
        let mut form = Form::new()
            .text("model", self.cfg.model.clone())
            .text("response_format", "verbose_json")
            .part("file", part);
        if let Some(lang) = &self.cfg.language {
            form = form.text("language", lang.clone());
        }

        debug!(model = %self.cfg.model, "sending Whisper transcription request");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.bytes().await?;
        if !status.is_success() {
            return Err(Error::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        let parsed: RawTranscript = serde_json::from_slice(&body)?;
        Ok(Transcript {
            text: parsed.text,
            language: parsed.language,
            duration_seconds: parsed.duration,
        })
    }
}

/// Wire shape of the `verbose_json` response. Only the fields we
/// surface are decoded — others are ignored.
#[derive(Debug, Deserialize)]
struct RawTranscript {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_groq_turbo() {
        let cfg = WhisperConfig::new("k");
        assert_eq!(cfg.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(cfg.model, "whisper-large-v3-turbo");
        assert!(cfg.language.is_none());
        assert_eq!(cfg.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn verbose_json_decodes_fully_populated_response() {
        let body = br#"{"text":"hello","language":"en","duration":1.23,"task":"transcribe"}"#;
        let parsed: RawTranscript = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed.text, "hello");
        assert_eq!(parsed.language.as_deref(), Some("en"));
        assert_eq!(parsed.duration, Some(1.23));
    }

    #[test]
    fn verbose_json_decodes_minimal_response() {
        // Some response shapes omit language / duration — make sure
        // we don't choke.
        let body = br#"{"text":"hello"}"#;
        let parsed: RawTranscript = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed.text, "hello");
        assert!(parsed.language.is_none());
        assert!(parsed.duration.is_none());
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        // Constructor must not perform any network I/O.
        let cfg = WhisperConfig::new("fake-key");
        let _client = WhisperClient::new(cfg).expect("client builds");
    }
}

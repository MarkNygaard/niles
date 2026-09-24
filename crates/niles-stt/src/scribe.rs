//! ElevenLabs Scribe transcription client.
//!
//! Not OpenAI-compatible, so not a base URL on [`crate::WhisperClient`]:
//! `POST /v1/speech-to-text`, an `xi-api-key` header rather than a
//! bearer token, `model_id` rather than `model`, and a response of
//! words rather than segments.
//!
//! Chosen for the two things Whisper does worst in a living room. It
//! mishears the wake word it is handed at the start of every command —
//! Myles, Charles, Nines — and Scribe takes key terms to listen for. And
//! Groq's Whisper reports no confidence at all, where Scribe reports a
//! log-probability for every word.

use crate::error::{Error, Result};
use crate::whisper::Transcript;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct ScribeConfig {
    pub api_key: String,
    /// Up to and including `/v1`.
    pub base_url: String,
    pub model: String,
    /// ISO-639-1 hint, e.g. `Some("en")`. None = auto-detect.
    pub language: Option<String>,
    /// Words to listen for: the assistant's own name above all.
    pub keyterms: Vec<String>,
    pub request_timeout: Duration,
}

pub struct ScribeClient {
    http: reqwest::Client,
    cfg: ScribeConfig,
}

impl ScribeClient {
    pub fn new(cfg: ScribeConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        Ok(Self { http, cfg })
    }

    pub async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<Transcript> {
        let url = format!("{}/speech-to-text", self.cfg.base_url.trim_end_matches('/'));
        let mut form = Form::new()
            .text("model_id", self.cfg.model.clone())
            .text("tag_audio_events", "false")
            .text("timestamps_granularity", "word")
            .part("file", Part::bytes(audio).file_name(filename.to_string()));
        if let Some(lang) = &self.cfg.language {
            form = form.text("language_code", lang.clone());
        }
        // A list field is the field repeated, as every multipart parser
        // reads one.
        for term in &self.cfg.keyterms {
            form = form.text("keyterms", term.clone());
        }

        debug!(model = %self.cfg.model, "sending Scribe transcription request");
        let resp = self
            .http
            .post(&url)
            .header("xi-api-key", &self.cfg.api_key)
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.bytes().await?;
        if !status.is_success() {
            const MAX_ERR_BODY: usize = 2048;
            let preview = &body[..body.len().min(MAX_ERR_BODY)];
            return Err(Error::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(preview).into_owned(),
            });
        }

        let parsed: RawScribe = serde_json::from_slice(&body)?;
        let scores = parsed.word_scores();
        // Logged rather than turned into a gate. The noise gate's
        // threshold was measured on Whisper's token log-probabilities;
        // these are per word, on a scale nobody here has read off a real
        // room yet, and a guessed threshold drops real commands. This
        // line is what the measurement will come from.
        info!(
            words = scores.count,
            min_logprob = scores.min,
            mean_logprob = scores.mean,
            language = parsed.language_code.as_deref().unwrap_or(""),
            language_probability = parsed.language_probability.unwrap_or(f64::NAN),
            "scribe scored {:?}",
            parsed.text
        );
        Ok(Transcript {
            text: parsed.text,
            language: parsed.language_code,
            duration_seconds: parsed.audio_duration_secs,
            confidence: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawScribe {
    text: String,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    language_probability: Option<f64>,
    #[serde(default)]
    audio_duration_secs: Option<f64>,
    #[serde(default)]
    words: Vec<RawWord>,
}

#[derive(Debug, Deserialize)]
struct RawWord {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    logprob: Option<f64>,
}

#[derive(Debug, PartialEq)]
struct WordScores {
    count: usize,
    min: f64,
    mean: f64,
}

impl RawScribe {
    /// Spoken words only: the spaces between them and tagged sounds carry
    /// a log-probability too, and neither says how sure it was of a word.
    fn word_scores(&self) -> WordScores {
        let scored: Vec<f64> = self
            .words
            .iter()
            .filter(|w| w.kind == "word")
            .filter_map(|w| w.logprob)
            .collect();
        if scored.is_empty() {
            return WordScores {
                count: 0,
                min: f64::NAN,
                mean: f64::NAN,
            };
        }
        WordScores {
            count: scored.len(),
            min: scored.iter().copied().fold(f64::INFINITY, f64::min),
            mean: scored.iter().sum::<f64>() / scored.len() as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] = br#"{
        "language_code": "eng",
        "language_probability": 0.98,
        "text": "Niles, turn off the lights.",
        "words": [
            {"text": "Niles,", "type": "word", "logprob": -0.2, "start": 0.1, "end": 0.5},
            {"text": " ", "type": "spacing", "logprob": -3.0, "start": 0.5, "end": 0.6},
            {"text": "turn", "type": "word", "logprob": -0.05, "start": 0.6, "end": 0.8},
            {"text": "(laughter)", "type": "audio_event", "logprob": -2.0}
        ],
        "transcription_id": "abc",
        "audio_duration_secs": 1.5
    }"#;

    #[test]
    fn the_response_is_read_off_the_wire() {
        let parsed: RawScribe = serde_json::from_slice(BODY).unwrap();
        assert_eq!(parsed.text, "Niles, turn off the lights.");
        assert_eq!(parsed.language_code.as_deref(), Some("eng"));
        assert_eq!(parsed.audio_duration_secs, Some(1.5));
    }

    #[test]
    fn only_spoken_words_are_scored() {
        // A space scored -3 is not a word it was unsure of.
        let parsed: RawScribe = serde_json::from_slice(BODY).unwrap();
        let scores = parsed.word_scores();
        assert_eq!(scores.count, 2);
        assert_eq!(scores.min, -0.2);
        assert!((scores.mean - -0.125).abs() < 1e-9);
    }

    #[test]
    fn nothing_heard_scores_nothing() {
        let parsed: RawScribe = serde_json::from_slice(br#"{"text": ""}"#).unwrap();
        assert_eq!(parsed.word_scores().count, 0);
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        let _ = ScribeClient::new(ScribeConfig {
            api_key: "fake".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "scribe_v2".into(),
            language: None,
            keyterms: vec!["Niles".into()],
            request_timeout: Duration::from_secs(5),
        })
        .expect("client builds");
    }
}

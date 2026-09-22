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

/// Successful transcription. Fields beyond `text` are best-effort:
/// Groq returns them today via `verbose_json` but a future provider
/// might not.
#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub duration_seconds: Option<f64>,
    /// What Whisper thought of its own answer.
    ///
    /// `None` from a provider that does not say, which is not the same
    /// as a confident answer and must not be read as one.
    pub confidence: Option<Confidence>,
}

/// Whisper's own opinion of whether that was speech.
///
/// A door closing near a satellite wakes it, and Whisper is then asked
/// to transcribe a room with nothing being said in it. It does not
/// return nothing — it returns a short, ordinary, entirely invented
/// sentence, because that is what a model trained to produce text does
/// with noise. "Thank you." is the classic one, and no rule about
/// sentence shape can tell that from somebody actually saying it.
///
/// These two numbers can. They are already on the wire — `verbose_json`
/// has always carried them and this crate decoded three fields and
/// dropped the rest — so reading them costs a struct, not a request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence {
    /// How sure Whisper is that the audio contained no speech at all.
    /// Nearer 1 is more sure there was nothing.
    pub no_speech_prob: f64,
    /// Mean log-probability of the tokens it chose. Less than about
    /// -1 means it was guessing.
    pub avg_logprob: f64,
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
            // Keep error bodies bounded — a multi-MB HTML error page
            // shouldn't ride into logs or anyhow chains.
            const MAX_ERR_BODY: usize = 2048;
            let preview = if body.len() > MAX_ERR_BODY {
                &body[..MAX_ERR_BODY]
            } else {
                &body[..]
            };
            return Err(Error::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(preview).into_owned(),
            });
        }

        let parsed: RawTranscript = serde_json::from_slice(&body)?;
        Ok(Transcript {
            text: parsed.text,
            language: parsed.language,
            duration_seconds: parsed.duration,
            confidence: confidence_of(&parsed.segments),
        })
    }
}

/// The whole utterance's confidence, from its segments.
///
/// Worst segment rather than an average: a wake word heard clearly
/// followed by three seconds of room would average out to something
/// reassuring, and it is the three seconds of room that decide whether
/// there was a command in there.
///
/// `None` when there are no segments, because a provider that says
/// nothing has not said the audio was fine.
fn confidence_of(segments: &[RawSegment]) -> Option<Confidence> {
    let no_speech_prob = segments
        .iter()
        .map(|s| s.no_speech_prob)
        .fold(f64::NEG_INFINITY, f64::max);
    let avg_logprob = segments
        .iter()
        .map(|s| s.avg_logprob)
        .fold(f64::INFINITY, f64::min);
    if segments.is_empty() {
        return None;
    }
    Some(Confidence {
        no_speech_prob,
        avg_logprob,
    })
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
    /// Absent from a provider that does not do `verbose_json`, and
    /// from a response with nothing in it.
    #[serde(default)]
    segments: Vec<RawSegment>,
}

#[derive(Debug, Deserialize)]
struct RawSegment {
    #[serde(default)]
    no_speech_prob: f64,
    /// Defaults to 0 rather than something damning: a provider that
    /// omits it has not reported a bad transcription, and treating
    /// silence as a low score would drop real speech.
    #[serde(default)]
    avg_logprob: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> WhisperConfig {
        WhisperConfig {
            api_key: "fake-key".into(),
            base_url: "https://example.invalid".into(),
            model: "test-model".into(),
            language: None,
            request_timeout: Duration::from_secs(5),
        }
    }

    fn seg(no_speech_prob: f64, avg_logprob: f64) -> RawSegment {
        RawSegment {
            no_speech_prob,
            avg_logprob,
        }
    }

    #[test]
    fn a_provider_that_says_nothing_is_not_a_provider_saying_it_was_fine() {
        // The distinction the caller depends on: absent means unknown,
        // and unknown must not read as confident.
        assert_eq!(confidence_of(&[]), None);
    }

    #[test]
    fn the_worst_segment_decides() {
        // A wake word heard clearly followed by three seconds of room
        // averages out to something reassuring, and it is the three
        // seconds of room that say whether a command was in there.
        let c = confidence_of(&[seg(0.01, -0.2), seg(0.93, -1.7)]).expect("some");
        assert_eq!(c.no_speech_prob, 0.93);
        assert_eq!(c.avg_logprob, -1.7);
    }

    #[test]
    fn segments_are_read_off_the_wire() {
        let body = br#"{"text":"Thank you.","segments":[
            {"id":0,"no_speech_prob":0.81,"avg_logprob":-1.4,"compression_ratio":1.1}
        ]}"#;
        let parsed: RawTranscript = serde_json::from_slice(body).unwrap();
        let c = confidence_of(&parsed.segments).expect("some");
        assert_eq!(c.no_speech_prob, 0.81);
        assert_eq!(c.avg_logprob, -1.4);
    }

    #[test]
    fn a_segment_missing_its_scores_is_not_treated_as_damning() {
        // A provider that omits them has not reported a bad
        // transcription, and reading silence as a low score would drop
        // real speech.
        let body = br#"{"text":"hello","segments":[{"id":0,"start":0.0,"end":1.0}]}"#;
        let parsed: RawTranscript = serde_json::from_slice(body).unwrap();
        let c = confidence_of(&parsed.segments).expect("some");
        assert_eq!(c.no_speech_prob, 0.0);
        assert_eq!(c.avg_logprob, 0.0);
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
        let _client = WhisperClient::new(test_cfg()).expect("client builds");
    }
}

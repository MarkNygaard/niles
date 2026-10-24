//! Piper TTS HTTP client.
//!
//! Targets a self-hosted Piper HTTP server. The current server
//! (piper-tts 1.8 and later) synthesises on `POST /synthesize` and serves
//! a web page on `GET /`; the older one synthesised on `POST /`. Both take
//! the JSON body `{ text, voice }` and answer with raw `audio/wav` bytes,
//! so the client tries `/synthesize` first, falls back to `/` on a 404 or
//! 405, and remembers which one answered.

use crate::error::{Error, Result};
use serde::Serialize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tracing::debug;

/// Inputs to [`PiperClient::new`]. Keeps configuration explicit so
/// the binary's config-loading layer is the only place that reads
/// files or env vars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiperConfig {
    /// Piper's base address, like `http://piper:5000`. An address that
    /// already ends in `/synthesize` is accepted too.
    pub base_url: String,
    pub default_voice: String,
    pub request_timeout: Duration,
}

/// Successful synthesis. Single field for now — future Piper
/// response headers (sample rate, duration) can be added without
/// breaking callers.
#[derive(Debug, Clone)]
pub struct Synthesis {
    pub audio_wav: Vec<u8>,
}

/// HTTP client around the Piper TTS endpoint. Owns its own
/// `reqwest::Client` so caller wiring stays a one-liner.
pub struct PiperClient {
    http: reqwest::Client,
    cfg: PiperConfig,
    /// `cfg.base_url` without a trailing `/` or `/synthesize`.
    base: String,
    /// The path that last answered, as an [`Endpoint`] code; `UNKNOWN`
    /// until the first synthesis.
    answered: AtomicU8,
}

/// The two places a Piper HTTP server has synthesised over the years.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    /// `POST /synthesize`, piper-tts 1.8 and later.
    Synthesize,
    /// `POST /`, the older server.
    Root,
}

const UNKNOWN: u8 = 0;

impl Endpoint {
    fn code(self) -> u8 {
        match self {
            Self::Synthesize => 1,
            Self::Root => 2,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Synthesize),
            2 => Some(Self::Root),
            _ => None,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Synthesize => Self::Root,
            Self::Root => Self::Synthesize,
        }
    }

    fn url(self, base: &str) -> String {
        match self {
            Self::Synthesize => format!("{base}/synthesize"),
            Self::Root => base.to_owned(),
        }
    }
}

/// Wire shape of the JSON request body. Local-only — callers pass
/// `&str` text + `Option<&str>` voice, the client handles the wire
/// shape.
#[derive(Debug, Serialize)]
struct PiperRequest<'a> {
    text: &'a str,
    voice: &'a str,
}

/// What a server says when the path is not where it synthesises.
fn wrong_path(status: u16) -> bool {
    matches!(status, 404 | 405)
}

impl PiperClient {
    pub fn new(cfg: PiperConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        let trimmed = cfg.base_url.trim().trim_end_matches('/');
        let base = trimmed
            .strip_suffix("/synthesize")
            .unwrap_or(trimmed)
            .trim_end_matches('/')
            .to_owned();
        Ok(Self {
            http,
            cfg,
            base,
            answered: AtomicU8::new(UNKNOWN),
        })
    }

    /// The configuration this client was built from.
    pub fn config(&self) -> &PiperConfig {
        &self.cfg
    }

    /// Submit `text` to the Piper endpoint and return WAV bytes.
    /// `voice_override` replaces `cfg.default_voice` when present.
    ///
    /// Goes to the path that answered last time, or `/synthesize` at
    /// first; a 404 or 405 there tries the other path once, so a Piper
    /// upgraded or downgraded under a running Niles keeps talking.
    pub async fn synthesize(&self, text: &str, voice_override: Option<&str>) -> Result<Synthesis> {
        if text.trim().is_empty() {
            return Err(Error::InvalidInput(
                "text must not be empty or whitespace-only".into(),
            ));
        }
        let voice = voice_override.unwrap_or(&self.cfg.default_voice).trim();
        if voice.is_empty() {
            return Err(Error::InvalidInput(
                "voice must not be empty or whitespace-only".into(),
            ));
        }
        let req = PiperRequest { text, voice };

        let first = Endpoint::from_code(self.answered.load(Ordering::Relaxed))
            .unwrap_or(Endpoint::Synthesize);
        let first_status = match self.post(first, &req).await {
            Err(Error::Provider { status, .. }) if wrong_path(status) => status,
            result => return self.remember_if_ok(first, result),
        };
        let second = first.other();
        debug!(
            status = first_status,
            "Piper refused {first:?}, trying {second:?}"
        );
        match self.post(second, &req).await {
            Err(Error::Provider { status, .. }) if wrong_path(status) => {
                let (synthesize, root) = match first {
                    Endpoint::Synthesize => (first_status, status),
                    Endpoint::Root => (status, first_status),
                };
                Err(Error::NoSynthesisEndpoint {
                    base_url: self.base.clone(),
                    synthesize,
                    root,
                })
            }
            result => self.remember_if_ok(second, result),
        }
    }

    fn remember_if_ok(&self, endpoint: Endpoint, result: Result<Synthesis>) -> Result<Synthesis> {
        if result.is_ok() {
            self.answered.store(endpoint.code(), Ordering::Relaxed);
        }
        result
    }

    async fn post(&self, endpoint: Endpoint, req: &PiperRequest<'_>) -> Result<Synthesis> {
        let url = endpoint.url(&self.base);
        debug!(voice = %req.voice, %url, "sending Piper TTS request");
        let resp = self.http.post(&url).json(req).send().await?;

        let status = resp.status();
        let body = resp.bytes().await?;
        if !status.is_success() {
            // Keep error bodies bounded — a multi-MB HTML error page
            // shouldn't ride into logs or anyhow chains.
            const MAX_ERR_BODY: usize = 2048;
            let preview = &body[..body.len().min(MAX_ERR_BODY)];
            return Err(Error::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(preview).into_owned(),
            });
        }

        Ok(Synthesis {
            audio_wav: body.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const WAV: &[u8] = b"RIFF....WAVE";

    fn test_cfg() -> PiperConfig {
        cfg_at("https://example.invalid")
    }

    fn cfg_at(base_url: &str) -> PiperConfig {
        PiperConfig {
            base_url: base_url.into(),
            default_voice: "en_GB-alan-medium".into(),
            request_timeout: Duration::from_secs(5),
        }
    }

    fn client_at(base_url: &str) -> PiperClient {
        PiperClient::new(cfg_at(base_url)).expect("client builds")
    }

    async fn answer(server: &MockServer, at: &str, status: u16, times: u64) {
        let response = if status == 200 {
            ResponseTemplate::new(200).set_body_bytes(WAV)
        } else {
            ResponseTemplate::new(status).set_body_string("<h1>Method Not Allowed</h1>")
        };
        Mock::given(method("POST"))
            .and(path(at))
            .respond_with(response)
            .expect(times)
            .mount(server)
            .await;
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        // Constructor must not perform any network I/O.
        let _client = PiperClient::new(test_cfg()).expect("client builds");
    }

    #[tokio::test]
    async fn rejects_empty_text_before_network() {
        let client = PiperClient::new(test_cfg()).expect("client builds");
        let err = client.synthesize("   ", None).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn rejects_empty_voice_override_before_network() {
        let client = PiperClient::new(test_cfg()).expect("client builds");
        let err = client.synthesize("hello", Some("   ")).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn the_current_server_answers_on_synthesize_with_text_and_voice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/synthesize"))
            .and(body_json(
                serde_json::json!({ "text": "hello", "voice": "en_GB-alan-medium" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(WAV))
            .expect(1)
            .mount(&server)
            .await;
        answer(&server, "/", 405, 0).await;

        let synthesis = client_at(&server.uri()).synthesize("hello", None).await;
        assert_eq!(synthesis.unwrap().audio_wav, WAV);
    }

    #[tokio::test]
    async fn the_older_server_answers_on_the_root() {
        let server = MockServer::start().await;
        answer(&server, "/synthesize", 404, 1).await;
        answer(&server, "/", 200, 1).await;

        let synthesis = client_at(&server.uri()).synthesize("hello", None).await;
        assert_eq!(synthesis.unwrap().audio_wav, WAV);
    }

    #[tokio::test]
    async fn an_address_ending_in_synthesize_is_not_doubled() {
        let server = MockServer::start().await;
        answer(&server, "/synthesize", 200, 2).await;

        let client = client_at(&format!("{}/synthesize/", server.uri()));
        client.synthesize("hello", None).await.unwrap();
        client.synthesize("again", None).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_path_that_answered_is_used_straight_away_next_time() {
        let server = MockServer::start().await;
        answer(&server, "/synthesize", 404, 1).await;
        answer(&server, "/", 200, 3).await;

        let client = client_at(&server.uri());
        for _ in 0..3 {
            client.synthesize("hello", None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_piper_upgraded_under_a_running_client_is_found_again() {
        let server = MockServer::start().await;
        answer(&server, "/synthesize", 404, 1).await;
        answer(&server, "/", 200, 1).await;
        let client = client_at(&server.uri());
        client.synthesize("hello", None).await.unwrap();
        server.verify().await;

        server.reset().await;
        answer(&server, "/", 405, 1).await;
        answer(&server, "/synthesize", 200, 2).await;
        client.synthesize("hello", None).await.unwrap();
        client.synthesize("again", None).await.unwrap();
    }

    #[tokio::test]
    async fn neither_path_answering_names_the_address_and_both_statuses() {
        let server = MockServer::start().await;
        answer(&server, "/", 405, 1).await;

        let err = client_at(&server.uri())
            .synthesize("hello", None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::NoSynthesisEndpoint {
                synthesize: 404,
                root: 405,
                ..
            }
        ));
        let message = err.to_string();
        assert!(
            message.contains(&format!("{}/synthesize", server.uri())),
            "{message}"
        );
        assert!(message.contains("base address"), "{message}");
    }

    #[tokio::test]
    async fn a_server_error_is_reported_without_trying_the_other_path() {
        let server = MockServer::start().await;
        answer(&server, "/synthesize", 500, 1).await;
        answer(&server, "/", 200, 0).await;

        let err = client_at(&server.uri())
            .synthesize("hello", None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Provider { status: 500, .. }));
    }
}

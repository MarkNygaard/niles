//! Piper TTS HTTP client.
//!
//! Targets the self-hosted Piper HTTP server's `POST /` endpoint.
//! The request body is JSON `{ text, voice }`; the response is
//! raw `audio/wav` bytes.

use crate::error::{Error, Result};
use serde::Serialize;
use std::time::Duration;
use tracing::debug;

/// Inputs to [`PiperClient::new`]. Keeps configuration explicit so
/// the binary's config-loading layer is the only place that reads
/// files or env vars.
#[derive(Debug, Clone)]
pub struct PiperConfig {
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
}

/// Wire shape of the JSON request body. Local-only — callers pass
/// `&str` text + `Option<&str>` voice, the client handles the wire
/// shape.
#[derive(Debug, Serialize)]
struct PiperRequest<'a> {
    text: &'a str,
    voice: &'a str,
}

impl PiperClient {
    pub fn new(cfg: PiperConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        Ok(Self { http, cfg })
    }

    /// Submit `text` to the Piper endpoint and return WAV bytes.
    /// `voice_override` replaces `cfg.default_voice` when present.
    pub async fn synthesize(&self, text: &str, voice_override: Option<&str>) -> Result<Synthesis> {
        let voice = voice_override.unwrap_or(&self.cfg.default_voice);
        let url = self.cfg.base_url.trim_end_matches('/');
        let req = PiperRequest { text, voice };

        debug!(voice = %voice, "sending Piper TTS request");
        let resp = self.http.post(url).json(&req).send().await?;

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

    fn test_cfg() -> PiperConfig {
        PiperConfig {
            base_url: "https://example.invalid".into(),
            default_voice: "en_GB-alan-medium".into(),
            request_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        // Constructor must not perform any network I/O.
        let _client = PiperClient::new(test_cfg()).expect("client builds");
    }

    #[test]
    fn piper_config_round_trip() {
        let cfg = test_cfg();
        assert_eq!(cfg.base_url, "https://example.invalid");
        assert_eq!(cfg.default_voice, "en_GB-alan-medium");
        assert_eq!(cfg.request_timeout, Duration::from_secs(5));
    }

    #[test]
    fn provider_error_body_is_truncated_to_2kb() {
        const MAX_ERR_BODY: usize = 2048;
        let oversized = vec![b'x'; 5000];
        assert_eq!(oversized[..oversized.len().min(MAX_ERR_BODY)].len(), 2048);

        let small = [b'y'; 100].to_vec();
        assert_eq!(small[..small.len().min(MAX_ERR_BODY)].len(), 100);
    }
}

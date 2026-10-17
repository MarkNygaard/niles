//! Kokoro-FastAPI's client: one line spoken at a time, in one host's
//! voice, as MP3.

use crate::error::{Error, Result};
use crate::{HTTP, Speech};
use std::time::Duration;

const SAY_WITHIN: Duration = Duration::from_secs(120);
/// A line of an 8-minute episode at most 16 MiB is nonsense: a server
/// streaming the wrong thing, caught before memory fills.
const MOST_LINE_AUDIO: usize = 16 << 20;
const VOICES_WITHIN: Duration = Duration::from_secs(10);
/// The pause before one retry of a line that timed out, dropped, or
/// answered 502/503/504.
const RETRY_AFTER: Duration = Duration::from_secs(2);

pub struct Kokoro {
    base: String,
}

impl Kokoro {
    pub fn new(address: &str) -> Result<Kokoro> {
        let parsed = url::Url::parse(address).map_err(|_| Error::Speech {
            reason: "the Kokoro address is not an address Niles can use".into(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(Error::Speech {
                reason: "the Kokoro address is not an address Niles can use".into(),
            });
        }
        Ok(Kokoro {
            base: address.trim_end_matches('/').to_string(),
        })
    }

    /// Kokoro the config points at, or what the add-on still needs.
    pub fn from_config(cfg: &niles_config::PodcastsConfig) -> Result<Kokoro> {
        match cfg.kokoro_address() {
            Some(address) => Kokoro::new(&address),
            None => Err(Error::NotSetUp {
                missing: vec!["Kokoro address"],
            }),
        }
    }

    /// The voices Kokoro offers, sorted; what came back is not a voice
    /// list when this is refused.
    pub async fn voices(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/audio/voices", self.base);
        let response = HTTP
            .get(&url)
            .timeout(VOICES_WITHIN)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    Error::Speech {
                        reason: format!(
                            "Kokoro did not answer within {} s",
                            VOICES_WITHIN.as_secs()
                        ),
                    }
                } else {
                    Error::Speech {
                        reason: "the request to Kokoro failed before it answered".into(),
                    }
                }
            })?;
        if !response.status().is_success() {
            return Err(Error::Speech {
                reason: format!("Kokoro answered with HTTP {}", response.status().as_u16()),
            });
        }
        let body: serde_json::Value = response.json().await.map_err(|_| Error::Speech {
            reason: "what came back is not Kokoro-FastAPI's voice list; is this its address?"
                .into(),
        })?;
        let list = body["voices"].as_array().ok_or_else(|| Error::Speech {
            reason: "what came back is not Kokoro-FastAPI's voice list; is this its address?"
                .into(),
        })?;
        let mut voices: Vec<String> = Vec::new();
        for item in list {
            let name = match item {
                serde_json::Value::String(name) => Some(name.clone()),
                serde_json::Value::Object(_) => item["name"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .map(str::to_string),
                _ => None,
            };
            let Some(name) = name else {
                return Err(Error::Speech {
                    reason: "what came back is not Kokoro-FastAPI's voice list; is this its \
                             address?"
                        .into(),
                });
            };
            if !voices.contains(&name) {
                voices.push(name);
            }
        }
        voices.sort();
        Ok(voices)
    }

    async fn say_once(&self, voice: &str, text: &str) -> Result<Vec<u8>> {
        let url = format!("{}/v1/audio/speech", self.base);
        let body = serde_json::json!({
            "model": "kokoro",
            "input": text,
            "voice": voice,
            "response_format": "mp3",
            "speed": 1.0,
            "stream": false,
        });
        let request = HTTP.post(&url).timeout(SAY_WITHIN).json(&body);
        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                return Err(if e.is_timeout() {
                    Error::Speech {
                        reason: format!("Kokoro did not answer within {} s", SAY_WITHIN.as_secs()),
                    }
                } else if e.is_connect() {
                    Error::Speech {
                        reason: "nothing answered at the Kokoro address; check it, and that Niles \
                                 can reach it"
                            .into(),
                    }
                } else {
                    Error::Speech {
                        reason: "the request to Kokoro failed before it answered".into(),
                    }
                });
            }
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // Transient answers are the caller's to retry once.
            return Err(Error::Speech {
                reason: match status {
                    502..=504 => format!("Kokoro answered with HTTP {status}"),
                    _ if status >= 500 => format!("Kokoro answered with HTTP {status}"),
                    _ => format!(
                        "Kokoro refused it (HTTP {status}); check the hosts' voices are ones it \
                         has"
                    ),
                },
            });
        }
        let mut audio = Vec::new();
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::Speech {
                reason: "the answer from Kokoro broke off".into(),
            })?;
            audio.extend_from_slice(&chunk);
            if audio.len() > MOST_LINE_AUDIO {
                return Err(Error::Speech {
                    reason: "Kokoro sent more audio than one line can be".into(),
                });
            }
        }
        if audio.is_empty() {
            return Err(Error::Speech {
                reason: "what came back is not MP3 audio".into(),
            });
        }
        Ok(audio)
    }
}

#[async_trait::async_trait]
impl Speech for Kokoro {
    async fn say(&self, voice: &str, text: &str) -> Result<Vec<u8>> {
        match self.say_once(voice, text).await {
            Ok(audio) => Ok(audio),
            Err(e) => {
                let retryable = match &e {
                    Error::Speech { reason } => {
                        reason.contains("did not answer within")
                            || reason.contains("nothing answered")
                            || reason.contains("answered with HTTP 502")
                            || reason.contains("answered with HTTP 503")
                            || reason.contains("answered with HTTP 504")
                            || reason.contains("broke off")
                    }
                    _ => false,
                };
                if retryable {
                    tokio::time::sleep(RETRY_AFTER).await;
                    self.say_once(voice, text).await
                } else {
                    Err(e)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const MP3: &[u8] = b"ID3\x00\x00\x00\x00\x00\x00\xff\xf3\x84\xc0";

    fn kokoro(server: &MockServer) -> Kokoro {
        Kokoro::new(&server.uri()).expect("a kokoro")
    }

    #[tokio::test]
    async fn the_request_asks_for_a_non_streamed_mp3_in_the_voice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(body_json(serde_json::json!({
                "model": "kokoro",
                "input": "Hello from Niles.",
                "voice": "af_heart",
                "response_format": "mp3",
                "speed": 1.0,
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(MP3))
            .expect(1)
            .mount(&server)
            .await;
        let audio = kokoro(&server)
            .say("af_heart", "Hello from Niles.")
            .await
            .expect("said");
        assert_eq!(audio, MP3);
    }

    #[tokio::test]
    async fn a_transient_answer_is_retried_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(MP3))
            .expect(1)
            .mount(&server)
            .await;
        kokoro(&server)
            .say("af_heart", "line")
            .await
            .expect("said after a retry");
    }

    #[tokio::test]
    async fn a_400_is_not_retried_and_points_at_the_voices() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;
        let error = kokoro(&server)
            .say("af_heart", "line")
            .await
            .expect_err("refused");
        let message = error.to_string();
        assert!(
            message.contains("refused") && message.contains("HTTP 400"),
            "unexpected: {message}"
        );
    }

    #[tokio::test]
    async fn voices_parse_from_strings_or_named_objects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/audio/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "voices": ["am_michael", "af_heart", { "name": "bm_george" }, { "id": "af_bella" }, "af_heart"]
            })))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            kokoro(&server).voices().await.expect("voices"),
            ["af_bella", "af_heart", "am_michael", "bm_george"]
        );
    }

    #[tokio::test]
    async fn an_html_answer_is_not_a_voice_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/audio/voices"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<html><body>hi</body></html>"),
            )
            .mount(&server)
            .await;
        let error = kokoro(&server).voices().await.expect_err("not a list");
        assert!(
            error.to_string().contains("is this its address"),
            "unexpected: {error}"
        );
    }

    #[tokio::test]
    async fn error_strings_never_carry_the_address() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let error = kokoro(&server)
            .say("af_heart", "line")
            .await
            .expect_err("failed");
        assert!(
            !error.to_string().contains(&server.uri()),
            "the message leaks: {error}"
        );
    }
}

//! Typing to Niles, from the app.
//!
//! The same Niles as the one in the kitchen: typed words go through the
//! same intents, tools and memory as spoken ones, so "add milk to the
//! list" means the same thing either way. Each signed-in person has
//! their own conversation, and Niles knows them by the voice their
//! address is linked to.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Longer than anybody types into a phone, short enough that a pasted
/// document is refused rather than sent to a model.
const MAX_LEN: usize = 2_000;

/// What answers the app's chat. Implemented by the binary, which owns
/// the dispatch this hands typed words to.
#[async_trait]
pub trait Chat: Send + Sync {
    /// Answer one message. `who` keeps one person's conversation apart
    /// from another's; `speaker` is the voice they are linked to, so
    /// Niles knows them as it would if they had said it.
    async fn reply(&self, who: &str, speaker: Option<&str>, text: &str) -> Result<String, String>;

    /// The conversation so far, oldest first. Empty once it has lapsed.
    fn history(&self, who: &str) -> Vec<Exchange>;

    /// Start over.
    fn forget(&self, who: &str);

    /// Words from a recording, by the same speech-to-text the satellites
    /// use. `filename` carries the format: `dictation.webm`, `.mp4`.
    async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Exchange {
    pub said: String,
    pub reply: String,
}

#[derive(Deserialize)]
pub struct Message {
    pub text: String,
}

#[derive(Serialize)]
pub struct Reply {
    pub reply: String,
}

#[derive(Serialize)]
pub struct Dictated {
    pub text: String,
}

/// A minute of compressed speech is well under a megabyte; this leaves
/// room for a phone that records at a generous bitrate.
pub const MAX_DICTATION_BYTES: usize = 8 * 1024 * 1024;

/// JSON, so the app can show why.
type Failure = (StatusCode, Json<serde_json::Value>);

/// `GET /chat` — the conversation so far.
pub async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Exchange>>, Failure> {
    let chat = chat(&state)?;
    let (who, _) = caller(&state, &headers);
    Ok(Json(chat.history(&who)))
}

/// `POST /chat` — say something, and get Niles's answer.
pub async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(message): Json<Message>,
) -> Result<Json<Reply>, Failure> {
    let chat = chat(&state)?;
    let text = message.text.trim();
    if text.is_empty() {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "say something first".into(),
        ));
    }
    if text.chars().count() > MAX_LEN {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("that is longer than {MAX_LEN} characters"),
        ));
    }
    let (who, speaker) = caller(&state, &headers);
    chat.reply(&who, speaker.as_deref(), text)
        .await
        .map(|reply| Json(Reply { reply }))
        .map_err(|e| failed(StatusCode::SERVICE_UNAVAILABLE, e))
}

/// `DELETE /chat` — start a new conversation.
pub async fn forget(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    let chat = chat(&state)?;
    let (who, _) = caller(&state, &headers);
    chat.forget(&who);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /chat/dictation` — a recording in, its words out.
///
/// Only the words come back, not an answer: they go in the message field
/// to be read over before sending, because a misheard word is easier to
/// fix there than after Niles has acted on it.
pub async fn dictation(
    State(state): State<AppState>,
    headers: HeaderMap,
    audio: Bytes,
) -> Result<Json<Dictated>, Failure> {
    let chat = chat(&state)?;
    if audio.is_empty() {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the recording was empty".into(),
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    chat.transcribe(audio.to_vec(), filename_for(content_type))
        .await
        .map(|text| Json(Dictated { text }))
        .map_err(|e| failed(StatusCode::BAD_GATEWAY, e))
}

/// The name a recording is sent to speech-to-text under, which is how
/// the provider learns its format. Chrome records WebM, Safari MP4.
fn filename_for(content_type: &str) -> &'static str {
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    match essence {
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "dictation.mp4",
        "audio/ogg" => "dictation.ogg",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "dictation.wav",
        "audio/mpeg" => "dictation.mp3",
        _ => "dictation.webm",
    }
}

fn chat(state: &AppState) -> Result<&Arc<dyn Chat>, Failure> {
    state.chat.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance has no chat".into(),
        )
    })
}

/// Whose conversation this is, and the voice they are linked to.
///
/// The gate has already let the request in, so somebody without a
/// session is the operator's token or a Niles with sign-in off. They
/// share one conversation between them, and nobody's profile.
fn caller(state: &AppState, headers: &HeaderMap) -> (String, Option<String>) {
    let Some(config) = state.config.as_ref().map(|c| c.current()) else {
        return (ANYONE.into(), None);
    };
    match crate::auth::signed_in_as(headers, &config) {
        Some(email) => {
            let speaker = config
                .auth
                .person_for(&email)
                .and_then(|p| p.speaker.clone());
            (email, speaker)
        }
        None => (ANYONE.into(), None),
    }
}

const ANYONE: &str = "anyone";

fn failed(status: StatusCode, message: String) -> Failure {
    (status, Json(serde_json::json!({ "error": message })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_core::{DeviceRegistry, EventBus};
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// Echoes, and remembers who it was talking to.
    #[derive(Default)]
    struct Echo {
        said: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl Chat for Echo {
        async fn reply(
            &self,
            who: &str,
            _speaker: Option<&str>,
            text: &str,
        ) -> Result<String, String> {
            self.said
                .lock()
                .unwrap()
                .push((who.to_string(), text.to_string()));
            Ok(format!("you said {text}"))
        }

        fn history(&self, who: &str) -> Vec<Exchange> {
            self.said
                .lock()
                .unwrap()
                .iter()
                .filter(|(w, _)| w == who)
                .map(|(_, text)| Exchange {
                    said: text.clone(),
                    reply: format!("you said {text}"),
                })
                .collect()
        }

        fn forget(&self, who: &str) {
            self.said.lock().unwrap().retain(|(w, _)| w != who);
        }

        async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<String, String> {
            Ok(format!("{} bytes of {filename}", audio.len()))
        }
    }

    fn app(chat: Option<Arc<dyn Chat>>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_chat(chat),
        )
    }

    async fn send(app: &axum::Router, method: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri("/chat")
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn answers_and_remembers_the_conversation() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, reply) = send(&app, "POST", Some(json!({ "text": " hello " }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said hello");

        let (_, history) = send(&app, "GET", None).await;
        assert_eq!(
            history,
            json!([{ "said": "hello", "reply": "you said hello" }])
        );

        let (status, _) = send(&app, "DELETE", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, history) = send(&app, "GET", None).await;
        assert_eq!(history, json!([]));
    }

    #[tokio::test]
    async fn an_empty_message_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, body) = send(&app, "POST", Some(json!({ "text": "   " }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn a_pasted_document_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let long = "word ".repeat(1_000);
        let (status, _) = send(&app, "POST", Some(json!({ "text": long }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    async fn dictate(
        app: &axum::Router,
        content_type: &str,
        audio: Vec<u8>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/chat/dictation")
            .header("content-type", content_type)
            .body(Body::from(audio))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn dictation_names_the_format_it_was_recorded_in() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, body) = dictate(&app, "audio/mp4", vec![0; 10]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["text"], "10 bytes of dictation.mp4");
        let (_, body) = dictate(&app, "audio/webm;codecs=opus", vec![0; 3]).await;
        assert_eq!(body["text"], "3 bytes of dictation.webm");
    }

    #[tokio::test]
    async fn dictation_takes_more_than_the_default_body_limit() {
        // axum refuses bodies over 2 MB unless told otherwise, and a
        // phone recording at a high bitrate gets there in a minute.
        let app = app(Some(Arc::new(Echo::default())));
        let (status, _) = dictate(&app, "audio/mp4", vec![0; 3 * 1024 * 1024]).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_empty_recording_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, _) = dictate(&app, "audio/webm", Vec::new()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn without_a_chat_it_says_so() {
        let app = app(None);
        let (status, _) = send(&app, "GET", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

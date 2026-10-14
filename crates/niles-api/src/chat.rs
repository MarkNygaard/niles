//! Typing to Niles, from the app — and reading back every conversation,
//! typed or spoken.
//!
//! The same Niles as the one in the kitchen: typed words go through the
//! same intents, tools and memory as spoken ones, so "add milk to the
//! list" means the same thing either way. Each signed-in person has
//! their own conversations, and Niles knows them by the voice their
//! address is linked to: what they said to a satellite is in their chat
//! too, and nobody else's.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Longer than anybody types into a phone, short enough that a pasted
/// document is refused rather than sent to a model.
const MAX_LEN: usize = 2_000;

/// Which conversation a message goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// The one going on, if there was a turn in the last few minutes;
    /// otherwise a new one.
    Latest,
    New,
    Continue(i64),
}

/// What answers the app's chat. Implemented by the binary, which owns
/// the dispatch this hands typed words to and the record it keeps.
///
/// `who` is the signed-in address and `speaker` the voice it is linked
/// to: the two together decide whose conversations these are, so what
/// somebody said to a satellite is theirs in the app too.
#[async_trait]
pub trait Chat: Send + Sync {
    /// Answer one message, in the conversation `into` names.
    async fn reply(
        &self,
        who: &str,
        speaker: Option<&str>,
        text: &str,
        into: Where,
    ) -> Result<Reply, String>;

    /// Their conversations, newest first.
    async fn conversations(&self, who: &str, speaker: Option<&str>) -> Vec<Summary>;

    /// One of theirs, every turn of it.
    async fn conversation(&self, who: &str, speaker: Option<&str>, id: i64)
    -> Option<Conversation>;

    /// Save, or not, the rule Niles read back in turn `turn` — the tap
    /// on the button under it.
    async fn answer(
        &self,
        who: &str,
        speaker: Option<&str>,
        id: i64,
        turn: usize,
        save: bool,
    ) -> Result<String, Unanswerable>;

    /// Forget one of theirs. False when there was none by that id.
    async fn delete(&self, who: &str, speaker: Option<&str>, id: i64) -> bool;

    /// Words from a recording, by the same speech-to-text the satellites
    /// use. `filename` carries the format: `dictation.webm`, `.mp4`.
    async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<String, String>;
}

/// Why a read-back could not be answered from the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unanswerable {
    NoSuchConversation,
    /// The turn asked nothing to save, or something came after it.
    NothingToAnswer,
}

/// One exchange: what was said, and what Niles answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub at: chrono::DateTime<chrono::Utc>,
    pub said: String,
    pub reply: String,
    /// `voice` or `app`.
    pub via: String,
    /// The room a spoken turn was said in.
    #[serde(default)]
    pub room: Option<String>,
    /// What wrote the reply when it was not Niles's own models: `claude`.
    #[serde(default)]
    pub by: Option<String>,
    /// The rule Niles read back and asked to save, in this turn.
    #[serde(default)]
    pub proposal: Option<Proposal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// "When the office door opens, turn off the office lights."
    pub sentence: String,
    /// The rule itself, so a tap hours later saves what was read back.
    /// Kept, never sent to the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub id: i64,
    pub title: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_at: chrono::DateTime<chrono::Utc>,
    pub turns: usize,
    /// Whether any of it was said to a satellite.
    pub spoken: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conversation {
    pub id: i64,
    pub title: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_at: chrono::DateTime<chrono::Utc>,
    pub turns: Vec<Turn>,
}

#[derive(Deserialize)]
pub struct Message {
    pub text: String,
    #[serde(default)]
    pub conversation_id: Option<i64>,
    #[serde(default)]
    pub new: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub reply: String,
    /// `claude` when Claude Code wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Why Claude Code did not answer, when it was switched on and the
    /// reply came from Niles's own models instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// The conversation it went into; absent where nothing is kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<i64>,
}

#[derive(Deserialize)]
pub struct Answer {
    pub turn: usize,
    pub save: bool,
}

#[derive(Serialize)]
pub struct Answered {
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

/// `GET /chat/conversations` — theirs, newest first.
pub async fn conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Summary>>, Failure> {
    let chat = chat(&state)?;
    let (who, speaker) = caller(&state, &headers);
    Ok(Json(chat.conversations(&who, speaker.as_deref()).await))
}

/// `GET /chat/conversations/{id}` — one of theirs.
pub async fn conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Conversation>, Failure> {
    let chat = chat(&state)?;
    let (who, speaker) = caller(&state, &headers);
    let mut conversation = chat
        .conversation(&who, speaker.as_deref(), id)
        .await
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such conversation".into()))?;
    // The rule a button would save stays on the server.
    for turn in &mut conversation.turns {
        if let Some(p) = &mut turn.proposal {
            p.rule = None;
        }
    }
    Ok(Json(conversation))
}

/// `POST /chat/conversations/{id}/proposal` — Save, or Don't save.
pub async fn answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(answer): Json<Answer>,
) -> Result<Json<Answered>, Failure> {
    let chat = chat(&state)?;
    let (who, speaker) = caller(&state, &headers);
    match chat
        .answer(&who, speaker.as_deref(), id, answer.turn, answer.save)
        .await
    {
        Ok(reply) => Ok(Json(Answered { reply })),
        Err(Unanswerable::NoSuchConversation) => {
            Err(failed(StatusCode::NOT_FOUND, "no such conversation".into()))
        }
        Err(Unanswerable::NothingToAnswer) => Err(failed(
            StatusCode::CONFLICT,
            "that has already been answered, or asked nothing to save".into(),
        )),
    }
}

/// `DELETE /chat/conversations/{id}`
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let chat = chat(&state)?;
    let (who, speaker) = caller(&state, &headers);
    if chat.delete(&who, speaker.as_deref(), id).await {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(failed(StatusCode::NOT_FOUND, "no such conversation".into()))
    }
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
    let into = match (message.conversation_id, message.new) {
        (Some(id), _) => Where::Continue(id),
        (None, true) => Where::New,
        (None, false) => Where::Latest,
    };
    chat.reply(&who, speaker.as_deref(), text, into)
        .await
        .map(Json)
        .map_err(|e| failed(StatusCode::SERVICE_UNAVAILABLE, e))
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

    /// Echoes, and keeps each caller's conversations as a real one would.
    #[derive(Default)]
    struct Echo {
        conversations: Mutex<Vec<(String, Conversation)>>,
    }

    fn turn(said: &str) -> Turn {
        Turn {
            at: chrono::Utc::now(),
            said: said.into(),
            reply: format!("you said {said}"),
            via: "app".into(),
            room: None,
            by: None,
            proposal: None,
        }
    }

    #[async_trait]
    impl Chat for Echo {
        async fn reply(
            &self,
            who: &str,
            _speaker: Option<&str>,
            text: &str,
            into: Where,
        ) -> Result<Reply, String> {
            let mut all = self.conversations.lock().unwrap();
            let target = match into {
                Where::Continue(id) => Some(id),
                Where::New => None,
                Where::Latest => all.iter().rev().find(|(w, _)| w == who).map(|(_, c)| c.id),
            };
            let id = match target.and_then(|id| {
                all.iter_mut()
                    .find(|(w, c)| w == who && c.id == id)
                    .map(|(_, c)| c)
            }) {
                Some(c) => {
                    c.turns.push(turn(text));
                    c.id
                }
                None => {
                    let id = all.len() as i64 + 1;
                    all.push((
                        who.to_string(),
                        Conversation {
                            id,
                            title: text.into(),
                            started_at: chrono::Utc::now(),
                            last_at: chrono::Utc::now(),
                            turns: vec![turn(text)],
                        },
                    ));
                    id
                }
            };
            Ok(Reply {
                reply: format!("you said {text}"),
                via: None,
                fallback: None,
                conversation_id: Some(id),
            })
        }

        async fn conversations(&self, who: &str, _speaker: Option<&str>) -> Vec<Summary> {
            self.conversations
                .lock()
                .unwrap()
                .iter()
                .rev()
                .filter(|(w, _)| w == who)
                .map(|(_, c)| Summary {
                    id: c.id,
                    title: c.title.clone(),
                    started_at: c.started_at,
                    last_at: c.last_at,
                    turns: c.turns.len(),
                    spoken: false,
                })
                .collect()
        }

        async fn conversation(
            &self,
            who: &str,
            _speaker: Option<&str>,
            id: i64,
        ) -> Option<Conversation> {
            self.conversations
                .lock()
                .unwrap()
                .iter()
                .find(|(w, c)| w == who && c.id == id)
                .map(|(_, c)| {
                    let mut c = c.clone();
                    if let Some(first) = c.turns.first_mut() {
                        first.proposal = Some(Proposal {
                            sentence: "When it is dark, turn on the porch.".into(),
                            rule: Some(json!({ "secret": "rule" })),
                        });
                    }
                    c
                })
        }

        async fn answer(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
            turn: usize,
            save: bool,
        ) -> Result<String, Unanswerable> {
            match (turn, save) {
                (0, true) => Ok("Saved.".into()),
                (0, false) => Ok("All right, I won't save it.".into()),
                _ => Err(Unanswerable::NothingToAnswer),
            }
        }

        async fn delete(&self, who: &str, _speaker: Option<&str>, id: i64) -> bool {
            let mut all = self.conversations.lock().unwrap();
            let before = all.len();
            all.retain(|(w, c)| !(w == who && c.id == id));
            all.len() != before
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
        call(app, method, "/chat", body).await
    }

    async fn call(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
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
    async fn answers_into_a_conversation_that_can_be_read_back() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, reply) = send(&app, "POST", Some(json!({ "text": " hello " }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said hello");
        let id = reply["conversation_id"].as_i64().unwrap();

        let (_, more) = send(
            &app,
            "POST",
            Some(json!({ "text": "and again", "conversation_id": id })),
        )
        .await;
        assert_eq!(more["conversation_id"], id);
        let (_, fresh) = send(&app, "POST", Some(json!({ "text": "new", "new": true }))).await;
        assert_ne!(fresh["conversation_id"], id);

        let (_, list) = call(&app, "GET", "/chat/conversations", None).await;
        assert_eq!(list.as_array().unwrap().len(), 2);
        let (_, one) = call(&app, "GET", &format!("/chat/conversations/{id}"), None).await;
        assert_eq!(one["turns"][1]["said"], "and again");
        // The read-back's sentence goes to the app; the rule a tap saves
        // does not.
        assert_eq!(
            one["turns"][0]["proposal"]["sentence"],
            "When it is dark, turn on the porch."
        );
        assert!(one["turns"][0]["proposal"].get("rule").is_none());

        let (status, _) = call(&app, "DELETE", &format!("/chat/conversations/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&app, "GET", &format!("/chat/conversations/{id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_read_back_is_answered_from_its_button() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, body) = call(
            &app,
            "POST",
            "/chat/conversations/1/proposal",
            Some(json!({ "turn": 0, "save": true })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["reply"], "Saved.");
        let (status, _) = call(
            &app,
            "POST",
            "/chat/conversations/1/proposal",
            Some(json!({ "turn": 3, "save": true })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
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
        let (status, _) = call(&app, "GET", "/chat/conversations", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

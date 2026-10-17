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
use axum::extract::{FromRequest, Path, State};
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
    /// Answer one message, in the conversation `into` names. The
    /// files on it were checked and prepared before they got here;
    /// a message with any always reaches the assistant.
    async fn reply(
        &self,
        who: &str,
        speaker: Option<&str>,
        text: &str,
        attachments: Vec<Upload>,
        into: Where,
    ) -> Result<Reply, String>;

    /// Their conversations, newest first.
    async fn conversations(&self, who: &str, speaker: Option<&str>) -> Vec<Summary>;

    /// One of theirs, every turn of it.
    async fn conversation(&self, who: &str, speaker: Option<&str>, id: i64)
    -> Option<Conversation>;

    /// One attachment of theirs, bytes and all. `None` when it is not
    /// theirs, not in that conversation, or not there at all.
    async fn attachment(
        &self,
        who: &str,
        speaker: Option<&str>,
        conversation: i64,
        id: i64,
    ) -> Option<Served>;

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

    /// Allow, or deny, what an approval card in conversation `id` is
    /// holding — the tap on one of its buttons. Answers what became of
    /// it, in a sentence.
    async fn decide(
        &self,
        who: &str,
        speaker: Option<&str>,
        id: i64,
        approval: &str,
        decision: Decision,
    ) -> Result<String, Undecidable>;

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

/// The answer to an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Allow once.
    Once,
    /// Allow for this conversation: the same tool runs without asking
    /// for the rest of it.
    Conversation,
    Deny,
}

/// Why an approval could not be answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undecidable {
    NoSuchConversation,
    /// No card by that id in the conversation.
    NoSuchApproval,
    /// It was answered already, lapsed, or Niles restarted since: the
    /// call it held is gone.
    NotWaiting,
    /// "Allow for this conversation" on something that is only ever
    /// approved once: a mail.
    OnceOnly,
}

/// Something Niles would not do on the model's word alone, waiting for
/// the person: shown as a card in the chat. What it holds — the tool
/// and its arguments — stays on the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalCard {
    pub id: String,
    /// A question: "Send this email?", "Let Linear create issue?"
    pub title: String,
    /// Whose account, or which add-on.
    pub subtitle: String,
    /// A brand mark's id (`linear`, `google`), or a generic icon's name.
    #[serde(default)]
    pub icon: Option<String>,
    /// What will happen, as labelled values: To, Subject; or a tool's
    /// arguments.
    #[serde(default)]
    pub fields: Vec<CardField>,
    /// A long text shown in full: a mail's body.
    #[serde(default)]
    pub body: Option<String>,
    /// Whether "Allow for this conversation" is offered.
    #[serde(default)]
    pub allow_conversation: bool,
    pub status: ApprovalStatus,
    /// What became of it, once it is not pending: "Sent", the error.
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Allowed,
    Denied,
    Lapsed,
    Failed,
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
    /// What wrote the reply when it was not Niles's own models: the
    /// assistant, `claude` or `chatgpt`.
    #[serde(default)]
    pub by: Option<String>,
    /// The rule Niles read back and asked to save, in this turn.
    #[serde(default)]
    pub proposal: Option<Proposal>,
    /// What this turn left waiting for the person's yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalCard>,
    /// The artifacts this turn made or changed, for a card under the
    /// reply. Brought up to date when the conversation is read back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<niles_artifacts::ArtifactRef>,
    /// The files the person attached to this turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// What kind of file is attached. More may follow, so a match on it
/// downstream must not assume these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AttachmentKind {
    Image,
    Pdf,
}

impl AttachmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Pdf => "pdf",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "image" => Some(Self::Image),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }
}

/// A file on a turn, as the app sees it. The bytes come from
/// `GET /chat/conversations/{id}/attachments/{a}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: i64,
    pub kind: AttachmentKind,
    /// What the person called it.
    pub name: String,
    /// Its size as kept, in bytes.
    pub size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// A file on its way in: checked and, for a picture, already written
/// again by `niles_artifacts::images`.
#[derive(Debug, Clone, PartialEq)]
pub struct Upload {
    pub kind: AttachmentKind,
    pub mime: String,
    pub name: String,
    pub bytes: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// One attachment on its way out.
#[derive(Debug, Clone, PartialEq)]
pub struct Served {
    pub kind: AttachmentKind,
    pub mime: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// "When the office door opens, turn off the office lights."
    pub sentence: String,
    /// The rule a line at a time, made from the rule itself: what Save
    /// saves, shown beside it. Absent from a turn kept before there was
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_full: Option<String>,
    /// What the rule will not do the way it reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    /// The assistant that wrote it, `claude` or `chatgpt`, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Why the assistant did not answer, when one was switched on and
    /// the reply came from Niles's own models instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Which assistant that was, so the app can name it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
    /// The conversation it went into; absent where nothing is kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<i64>,
    /// The files that went with the message, as kept: shown on the
    /// bubble without waiting for a refetch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
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
pub(crate) type Failure = (StatusCode, Json<serde_json::Value>);

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

#[derive(Deserialize)]
pub struct Decide {
    pub decision: Decision,
}

/// `POST /chat/conversations/{id}/approvals/{approval}` — Allow once,
/// Allow for this conversation, or Deny.
pub async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, approval)): Path<(i64, String)>,
    Json(body): Json<Decide>,
) -> Result<Json<Answered>, Failure> {
    let chat = chat(&state)?;
    let (who, speaker) = caller(&state, &headers);
    match chat
        .decide(&who, speaker.as_deref(), id, &approval, body.decision)
        .await
    {
        Ok(reply) => Ok(Json(Answered { reply })),
        Err(Undecidable::NoSuchConversation) => {
            Err(failed(StatusCode::NOT_FOUND, "no such conversation".into()))
        }
        Err(Undecidable::NoSuchApproval) => {
            Err(failed(StatusCode::NOT_FOUND, "no such approval".into()))
        }
        Err(Undecidable::NotWaiting) => Err(failed(
            StatusCode::CONFLICT,
            "that is no longer waiting: it was answered, or it lapsed".into(),
        )),
        Err(Undecidable::OnceOnly) => Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "this can only be allowed once".into(),
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
///
/// Two body shapes: the usual JSON, and `multipart/form-data` carrying
/// `text`, `conversation_id`, `new` and repeated `files` parts when the
/// message has files on it.
pub async fn send(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Result<Json<Reply>, Failure> {
    let chat = chat(&state)?;
    let headers = request.headers().clone();
    let multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.trim_start().starts_with("multipart/form-data"));
    let (message, uploads) = if multipart {
        crate::chat_attachments::read(request, &state).await?
    } else {
        let Json(message) = Json::<Message>::from_request(request, &state)
            .await
            .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
        (message, Vec::new())
    };
    // `headers` was cloned before the body was consumed: `caller`
    // below still needs it.
    let text = message.text.trim();
    if text.is_empty() && uploads.is_empty() {
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
    chat.reply(&who, speaker.as_deref(), text, uploads, into)
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

pub(crate) fn chat(state: &AppState) -> Result<&Arc<dyn Chat>, Failure> {
    state.chat.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance has no chat".into(),
        )
    })
}

/// The key the caller's conversations, and everything else of theirs,
/// are kept under.
pub(crate) fn person_of(state: &AppState, headers: &HeaderMap) -> String {
    let (who, speaker) = caller(state, headers);
    niles_training::person_key(speaker.as_deref(), &who)
}

/// Whose conversation this is, and the voice they are linked to.
///
/// The gate has already let the request in, so somebody without a
/// session is the operator's token or a Niles with sign-in off. They
/// share one conversation between them, and nobody's profile.
pub(crate) fn caller(state: &AppState, headers: &HeaderMap) -> (String, Option<String>) {
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

pub(crate) fn failed(status: StatusCode, message: String) -> Failure {
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
    /// Records the uploads and the target of the last `reply`, for the
    /// multipart tests.
    #[derive(Default)]
    struct Echo {
        conversations: Mutex<Vec<(String, Conversation)>>,
        uploads: Mutex<Vec<Upload>>,
        last_where: Mutex<Option<Where>>,
        /// `(conversation, id)` the stub says it owns, and the file
        /// there: both must match the ask, the way the real check
        /// matches person, conversation and id.
        served: Mutex<Option<Served>>,
        served_for: Mutex<Option<(i64, i64)>>,
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
            approval: None,
            artifacts: Vec::new(),
            attachments: Vec::new(),
        }
    }

    #[async_trait]
    impl Chat for Echo {
        async fn reply(
            &self,
            who: &str,
            _speaker: Option<&str>,
            text: &str,
            attachments: Vec<Upload>,
            into: Where,
        ) -> Result<Reply, String> {
            *self.uploads.lock().unwrap() = attachments;
            *self.last_where.lock().unwrap() = Some(into);
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
                fallback_from: None,
                conversation_id: Some(id),
                attachments: Vec::new(),
            })
        }

        async fn attachment(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            conversation: i64,
            id: i64,
        ) -> Option<Served> {
            if self.served_for.lock().unwrap().as_ref() == Some(&(conversation, id)) {
                return self.served.lock().unwrap().clone();
            }
            None
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
                            in_full: Some(
                                "When:\n  1. The sun sets\nDo:\n  Turn on the porch lights".into(),
                            ),
                            warnings: Vec::new(),
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

        async fn decide(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            id: i64,
            approval: &str,
            decision: Decision,
        ) -> Result<String, Undecidable> {
            match (id, approval, decision) {
                (1, "a1", Decision::Once) => Ok("Done.".into()),
                (1, "a1", Decision::Deny) => Ok("All right, I didn't.".into()),
                (1, "mail", Decision::Conversation) => Err(Undecidable::OnceOnly),
                (1, "gone", _) => Err(Undecidable::NotWaiting),
                (1, _, _) => Err(Undecidable::NoSuchApproval),
                _ => Err(Undecidable::NoSuchConversation),
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
    async fn an_approval_is_answered_with_its_decision() {
        let app = app(Some(Arc::new(Echo::default())));
        let decide = |approval: &str, decision: &str| {
            let app = app.clone();
            let uri = format!("/chat/conversations/1/approvals/{approval}");
            let body = json!({ "decision": decision });
            async move { call(&app, "POST", &uri, Some(body)).await }
        };
        let (status, body) = decide("a1", "once").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["reply"], "Done.");
        assert_eq!(decide("gone", "once").await.0, StatusCode::CONFLICT);
        assert_eq!(
            decide("mail", "conversation").await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(decide("nope", "deny").await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            decide("a1", "maybe").await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
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
        // The read-back's sentence and the rule in full go to the app;
        // the rule a tap saves does not.
        assert_eq!(
            one["turns"][0]["proposal"]["sentence"],
            "When it is dark, turn on the porch."
        );
        assert!(
            one["turns"][0]["proposal"]["in_full"]
                .as_str()
                .unwrap()
                .contains("Turn on the porch lights")
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

    // A real JPEG, tiny; its SOF0 dimensions patched when a test wants
    // a header the decoder will refuse.
    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([9, 9, 9, 255]),
        ));
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image)
            .unwrap();
        if (width, height) != (2, 2) {
            let i = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
            // SOF0: length, precision, height, width — two bytes each.
            bytes[i + 5..i + 7].copy_from_slice(&(height as u16).to_be_bytes());
            bytes[i + 7..i + 9].copy_from_slice(&(width as u16).to_be_bytes());
        }
        bytes
    }

    /// `(name, filename, bytes)` parts as one multipart body.
    fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
        let boundary = "niles-test-boundary";
        let mut body = Vec::new();
        for (name, filename, bytes) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            let disposition = match filename {
                Some(f) => {
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"")
                }
                None => format!("Content-Disposition: form-data; name=\"{name}\""),
            };
            body.extend_from_slice(disposition.as_bytes());
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    async fn send_files(
        app: &axum::Router,
        parts: &[(&str, Option<&str>, &[u8])],
    ) -> (StatusCode, Value) {
        let (content_type, body) = multipart(parts);
        let request = Request::builder()
            .method("POST")
            .uri("/chat")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// One attachment GET, with its headers: the body is bytes, not
    /// JSON.
    async fn fetch_attachment(app: &axum::Router, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    #[tokio::test]
    async fn a_message_with_a_picture_is_answered() {
        let echo = Arc::new(Echo::default());
        let app = app(Some(echo.clone()));
        let (status, reply) = send_files(
            &app,
            &[
                ("text", None, b"what is this"),
                ("files", Some("plant.jpg"), &jpeg(2, 2)),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said what is this");
        let uploads = echo.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].kind, AttachmentKind::Image);
        assert_eq!(uploads[0].mime, "image/jpeg");
        assert_eq!(uploads[0].name, "plant.jpg");
        assert_eq!(uploads[0].width, Some(2));
        assert_eq!(uploads[0].height, Some(2));
    }

    #[tokio::test]
    async fn a_pdf_is_kept_byte_for_byte() {
        let echo = Arc::new(Echo::default());
        let app = app(Some(echo.clone()));
        let pdf = b"%PDF-1.7\nfake but well-formed\n%%EOF";
        let (status, _) = send_files(
            &app,
            &[
                ("text", None, b"file this"),
                ("files", Some("ticket.pdf"), pdf),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let uploads = echo.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].kind, AttachmentKind::Pdf);
        assert_eq!(uploads[0].mime, "application/pdf");
        assert_eq!(uploads[0].bytes, pdf.to_vec());
        assert_eq!(uploads[0].width, None);
    }

    #[tokio::test]
    async fn a_picture_on_its_own_is_a_message() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, reply) = send_files(&app, &[("files", Some("p.jpg"), &jpeg(2, 2))]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said ");
    }

    #[tokio::test]
    async fn a_sixth_file_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let pic = jpeg(2, 2);
        let mut parts: Vec<(&str, Option<&str>, &[u8])> = vec![("text", None, b"look at these")];
        for _ in 0..6 {
            parts.push(("files", Some("p.jpg"), &pic));
        }
        let (status, body) = send_files(&app, &parts).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("Five files at most")
        );
    }

    #[tokio::test]
    async fn a_file_over_ten_megabytes_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let big = vec![0u8; 11 * 1024 * 1024];
        let (status, body) = send_files(
            &app,
            &[("text", None, b"look"), ("files", Some("big.jpg"), &big)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let error = body["error"].as_str().unwrap();
        assert!(error.contains("10 MB"), "{error}");
        assert!(error.contains("big.jpg"), "{error}");
    }

    #[tokio::test]
    async fn a_picture_header_claiming_a_huge_size_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let bomb = jpeg(20_000, 20_000);
        let (status, body) = send_files(
            &app,
            &[("text", None, b"look"), ("files", Some("bomb.jpg"), &bomb)],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("smaller picture"));
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_picture_or_pdf_is_refused() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, body) = send_files(
            &app,
            &[
                ("text", None, b"look"),
                ("files", Some("page.html"), b"<html><body>hi</body></html>"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("not a JPEG, PNG, WebP, GIF or PDF")
        );
    }

    #[tokio::test]
    async fn a_heic_photo_is_refused_with_what_to_change() {
        let app = app(Some(Arc::new(Echo::default())));
        let mut heic = Vec::new();
        heic.extend_from_slice(&24u32.to_be_bytes());
        heic.extend_from_slice(b"ftypheic");
        heic.extend_from_slice(&0u32.to_be_bytes());
        heic.extend_from_slice(b"heicmif1");
        let (status, body) = send_files(
            &app,
            &[
                ("text", None, b"look"),
                ("files", Some("photo.heic"), &heic),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let error = body["error"].as_str().unwrap();
        assert!(error.contains("HEIC"), "{error}");
        assert!(error.contains("Most Compatible"), "{error}");
    }

    #[tokio::test]
    async fn conversation_id_and_new_are_read_off_the_form() {
        let echo = Arc::new(Echo::default());
        let app = app(Some(echo.clone()));
        let (status, _) = send_files(
            &app,
            &[
                ("text", None, b"hello"),
                ("conversation_id", None, b"7"),
                ("files", Some("p.jpg"), &jpeg(2, 2)),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*echo.last_where.lock().unwrap(), Some(Where::Continue(7)));

        let (status, _) =
            send_files(&app, &[("text", None, b"hello"), ("new", None, b"true")]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*echo.last_where.lock().unwrap(), Some(Where::New));
    }

    #[tokio::test]
    async fn a_multipart_conversation_id_that_is_not_a_number_is_a_422() {
        let app = app(Some(Arc::new(Echo::default())));
        let (status, body) = send_files(
            &app,
            &[("text", None, b"hello"), ("conversation_id", None, b"soon")],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "conversation_id must be a number");
    }

    #[tokio::test]
    async fn a_field_the_app_does_not_know_yet_is_skipped() {
        let echo = Arc::new(Echo::default());
        let app = app(Some(echo.clone()));
        let (status, reply) = send_files(
            &app,
            &[
                ("text", None, b"hello"),
                ("future-thing", None, b"whatever"),
                ("files", Some("p.jpg"), &jpeg(2, 2)),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said hello");
        assert_eq!(echo.uploads.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_json_body_is_answered_as_before() {
        let echo = Arc::new(Echo::default());
        let app = app(Some(echo.clone()));
        let (status, reply) = send(&app, "POST", Some(json!({ "text": "hello" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["reply"], "you said hello");
        assert!(echo.uploads.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_attachment_is_served_to_its_owner_with_picture_headers() {
        let echo = Arc::new(Echo::default());
        *echo.served.lock().unwrap() = Some(Served {
            kind: AttachmentKind::Image,
            mime: "image/jpeg".into(),
            name: "the plant.jpg".into(),
            bytes: vec![1, 2, 3],
        });
        *echo.served_for.lock().unwrap() = Some((1, 7));
        let app = app(Some(echo));
        let (status, headers, bytes) =
            fetch_attachment(&app, "/chat/conversations/1/attachments/7").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, vec![1, 2, 3]);
        assert_eq!(headers["content-type"], "image/jpeg");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(
            headers["content-security-policy"],
            "default-src 'none'; sandbox"
        );
        assert_eq!(
            headers["cache-control"],
            "private, max-age=31536000, immutable"
        );
        assert_eq!(headers["cross-origin-resource-policy"], "same-origin");
        // The name is percent-encoded in the disposition, so a quote or
        // a space in it cannot split or mangle the header.
        let disposition = headers["content-disposition"].to_str().unwrap();
        assert!(disposition.starts_with("inline;"));
        assert!(disposition.contains("the%20plant.jpg"), "{disposition}");
    }

    #[tokio::test]
    async fn a_pdf_is_served_as_a_pdf_with_the_same_headers() {
        let echo = Arc::new(Echo::default());
        *echo.served.lock().unwrap() = Some(Served {
            kind: AttachmentKind::Pdf,
            mime: "application/pdf".into(),
            name: "ticket.pdf".into(),
            bytes: b"%PDF-1.7".to_vec(),
        });
        *echo.served_for.lock().unwrap() = Some((1, 3));
        let app = app(Some(echo));
        let (status, headers, bytes) =
            fetch_attachment(&app, "/chat/conversations/1/attachments/3").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "application/pdf");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(bytes, b"%PDF-1.7".to_vec());
        assert!(
            headers["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("inline;")
        );
    }

    #[tokio::test]
    async fn an_attachment_is_not_found_under_another_conversation_or_another_person() {
        let echo = Arc::new(Echo::default());
        *echo.served.lock().unwrap() = Some(Served {
            kind: AttachmentKind::Image,
            mime: "image/jpeg".into(),
            name: "a.jpg".into(),
            bytes: vec![1],
        });
        *echo.served_for.lock().unwrap() = Some((1, 7));
        let app = app(Some(echo));
        // The stub only owns conversation 1: another of its own
        // conversations, and the wrong id entirely, are both 404.
        let (status, _, _) = fetch_attachment(&app, "/chat/conversations/2/attachments/7").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = fetch_attachment(&app, "/chat/conversations/1/attachments/8").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_turn_with_attachments_round_trips_and_one_without_omits_them() {
        let mut kept = turn("look at this");
        kept.attachments = vec![Attachment {
            id: 3,
            kind: AttachmentKind::Image,
            name: "plant.jpg".into(),
            size: 1234,
            width: Some(1600),
            height: Some(1200),
        }];
        let json = serde_json::to_string(&kept).unwrap();
        let back: Turn = serde_json::from_str(&json).unwrap();
        assert_eq!(kept, back);

        let bare = serde_json::to_string(&turn("just words")).unwrap();
        assert!(!bare.contains("attachments"), "{bare}");
    }
}

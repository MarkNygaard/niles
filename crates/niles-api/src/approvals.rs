//! What waits for the signed-in person's yes, and what they decided:
//! Approvals, in Niles's panel and on My profile.
//!
//! The same held calls the cards in the chat answer, listed in one
//! place, and answered by id with the same decision a card's button
//! sends. Only the caller's own: whose they are is the session's, as
//! their conversations are, never a name in the path.

use crate::chat::{Answered, Decide, Decision, Failure, Undecidable, caller, failed, undecided};
use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Answered by the binary, which holds the calls and keeps the log.
#[async_trait]
pub trait Approvals: Send + Sync {
    /// Theirs still waiting, newest first.
    async fn waiting(&self, who: &str, speaker: Option<&str>) -> Vec<Waiting>;

    /// Answer one by its id, as its card's button would. `device` is the
    /// push address of the device it was answered on, if any.
    async fn decide(
        &self,
        who: &str,
        speaker: Option<&str>,
        approval: &str,
        decision: Decision,
        device: Option<&str>,
    ) -> Result<String, Undecidable>;

    /// A page of theirs decided or lapsed, newest first, all before the
    /// entry `before` when it is given.
    async fn history(
        &self,
        who: &str,
        speaker: Option<&str>,
        before: Option<i64>,
    ) -> Result<History, String>;
}

/// One held call, as the panel draws it: the card the chat shows, and
/// where and when it was asked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Waiting {
    #[serde(flatten)]
    pub card: crate::chat::ApprovalCard,
    /// The add-on it is for, as a person reads it: "Linear", "Mail".
    pub tool: String,
    /// What it would do, in a line.
    pub summary: String,
    /// The conversation its card is in; none for a question asked aloud.
    pub conversation_id: Option<i64>,
    pub asked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// `app` or `voice`.
    pub via: String,
}

#[derive(Debug, Serialize)]
pub struct WaitingList {
    pub approvals: Vec<Waiting>,
}

/// One decided, or lapsed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decided {
    /// Its place in the log, which `before` pages by.
    pub id: i64,
    /// The card's id.
    pub approval: String,
    pub tool: String,
    pub icon: Option<String>,
    pub title: String,
    pub summary: String,
    /// `allowed`, `denied`, `lapsed` or `failed`.
    pub status: String,
    /// `once`, `conversation` or `deny`; none for a lapse.
    pub decision: Option<String>,
    pub outcome: Option<String>,
    /// Where it was answered: `app` or `voice`; none for a lapse.
    pub decided_via: Option<String>,
    pub asked_via: String,
    pub asked_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub conversation_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct History {
    pub entries: Vec<Decided>,
    /// What to ask for the next page with, while there is one.
    pub next: Option<i64>,
    /// False where there is no database: nothing is kept, and the app
    /// says so rather than showing an empty history as if it were one.
    pub kept: bool,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub before: Option<i64>,
}

fn approvals(state: &AppState) -> Result<&Arc<dyn Approvals>, Failure> {
    state.approvals.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance holds nothing for approval".into(),
        )
    })
}

/// `GET /me/approvals` — theirs still waiting.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<WaitingList>, Failure> {
    let approvals = approvals(&state)?;
    let (who, speaker) = caller(&state, &headers);
    Ok(Json(WaitingList {
        approvals: approvals.waiting(&who, speaker.as_deref()).await,
    }))
}

/// `POST /me/approvals/{approval}` — Allow once, Allow for this
/// conversation, or Deny, from the panel.
pub async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(approval): Path<String>,
    Json(body): Json<Decide>,
) -> Result<Json<Answered>, Failure> {
    let approvals = approvals(&state)?;
    let (who, speaker) = caller(&state, &headers);
    approvals
        .decide(
            &who,
            speaker.as_deref(),
            &approval,
            body.decision,
            body.device.as_deref(),
        )
        .await
        .map(|reply| Json(Answered { reply }))
        .map_err(undecided)
}

/// `GET /me/approvals/history?before=` — theirs decided, a page at a
/// time.
pub async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<History>, Failure> {
    let approvals = approvals(&state)?;
    let (who, speaker) = caller(&state, &headers);
    approvals
        .history(&who, speaker.as_deref(), page.before)
        .await
        .map(Json)
        .map_err(|e| failed(StatusCode::BAD_GATEWAY, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ApprovalCard, ApprovalStatus};
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_core::{DeviceRegistry, EventBus};
    use parking_lot::Mutex;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// One waiting call, `a1`, and what was asked of it.
    #[derive(Default)]
    struct Held {
        decided: Mutex<Vec<(String, Decision, Option<String>)>>,
        asked_before: Mutex<Option<Option<i64>>>,
    }

    fn waiting() -> Waiting {
        Waiting {
            card: ApprovalCard {
                id: "a1".into(),
                title: "Let Linear create issue?".into(),
                subtitle: "Linear".into(),
                icon: Some("linear".into()),
                fields: Vec::new(),
                body: None,
                allow_conversation: true,
                status: ApprovalStatus::Pending,
                outcome: None,
            },
            tool: "Linear".into(),
            summary: "Title: WLED presets".into(),
            conversation_id: Some(4),
            asked_at: Utc::now(),
            expires_at: Utc::now(),
            via: "app".into(),
        }
    }

    #[async_trait]
    impl Approvals for Held {
        async fn waiting(&self, _who: &str, _speaker: Option<&str>) -> Vec<Waiting> {
            vec![waiting()]
        }

        async fn decide(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            approval: &str,
            decision: Decision,
            device: Option<&str>,
        ) -> Result<String, Undecidable> {
            self.decided
                .lock()
                .push((approval.to_string(), decision, device.map(str::to_string)));
            match (approval, decision) {
                ("a1", Decision::Deny) => Ok("All right, I didn't.".into()),
                ("a1", _) => Ok("Done.".into()),
                ("mail", Decision::Conversation) => Err(Undecidable::OnceOnly),
                ("gone", _) => Err(Undecidable::NotWaiting),
                _ => Err(Undecidable::NoSuchApproval),
            }
        }

        async fn history(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            before: Option<i64>,
        ) -> Result<History, String> {
            *self.asked_before.lock() = Some(before);
            Ok(History {
                entries: Vec::new(),
                next: before.map(|b| b - 30),
                kept: true,
            })
        }
    }

    fn app(approvals: Option<Arc<dyn Approvals>>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_approvals(approvals),
        )
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
    async fn what_waits_is_listed_as_its_card_and_when() {
        let app = app(Some(Arc::new(Held::default())));
        let (status, body) = call(&app, "GET", "/me/approvals", None).await;
        assert_eq!(status, StatusCode::OK);
        let one = &body["approvals"][0];
        assert_eq!(one["id"], "a1");
        assert_eq!(one["title"], "Let Linear create issue?");
        assert_eq!(one["tool"], "Linear");
        assert_eq!(one["summary"], "Title: WLED presets");
        assert_eq!(one["conversation_id"], 4);
        assert_eq!(one["via"], "app");
        assert_eq!(one["status"], "pending");
        assert!(one["expires_at"].is_string());
    }

    #[tokio::test]
    async fn one_is_answered_by_its_id_as_its_card_would_be() {
        let held = Arc::new(Held::default());
        let app = app(Some(held.clone()));
        let decide = |approval: &str, decision: &str| {
            let app = app.clone();
            let uri = format!("/me/approvals/{approval}");
            let body = json!({ "decision": decision });
            async move { call(&app, "POST", &uri, Some(body)).await }
        };
        let (status, body) = decide("a1", "conversation").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["reply"], "Done.");
        assert_eq!(
            decide("a1", "deny").await.1["reply"],
            "All right, I didn't."
        );
        assert_eq!(decide("gone", "once").await.0, StatusCode::CONFLICT);
        assert_eq!(
            decide("mail", "conversation").await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            decide("someone-elses", "once").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            decide("a1", "maybe").await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            held.decided.lock()[..2],
            [
                ("a1".to_string(), Decision::Conversation, None),
                ("a1".to_string(), Decision::Deny, None)
            ]
        );
    }

    #[tokio::test]
    async fn the_device_it_was_answered_on_is_passed_on() {
        let held = Arc::new(Held::default());
        let app = app(Some(held.clone()));
        let (status, _) = call(
            &app,
            "POST",
            "/me/approvals/a1",
            Some(json!({
                "decision": "deny",
                "device": "https://push.example/1",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            held.decided.lock().last().unwrap(),
            &(
                "a1".to_string(),
                Decision::Deny,
                Some("https://push.example/1".to_string())
            )
        );
    }

    #[tokio::test]
    async fn history_is_asked_for_a_page_at_a_time() {
        let held = Arc::new(Held::default());
        let app = app(Some(held.clone()));
        let (status, body) = call(&app, "GET", "/me/approvals/history", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["kept"], true);
        assert_eq!(body["next"], Value::Null);
        assert_eq!(*held.asked_before.lock(), Some(None));

        let (_, body) = call(&app, "GET", "/me/approvals/history?before=90", None).await;
        assert_eq!(*held.asked_before.lock(), Some(Some(90)));
        assert_eq!(body["next"], 60);
    }

    #[tokio::test]
    async fn without_approvals_it_says_so() {
        let app = app(None);
        let (status, _) = call(&app, "GET", "/me/approvals", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

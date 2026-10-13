//! The skills Niles has made, from the app.
//!
//! The same store the voice tools and the background review write to.
//! Skills are learned without anybody watching, so this is where the
//! household sees what Niles taught itself — and pins, archives or
//! deletes what it should not have.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use niles_skills::{Error, Provenance, Skill, SkillStatus, SkillStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type Failure = (StatusCode, String);

#[derive(Debug, Serialize)]
pub struct SkillView {
    pub name: String,
    pub description: String,
    pub version: String,
    pub status: SkillStatus,
    pub provenance: Provenance,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_viewed_at: Option<DateTime<Utc>>,
    pub last_patched_at: Option<DateTime<Utc>>,
    pub usage_count: u64,
    pub view_count: u64,
    pub patch_count: u64,
    /// Only when one skill is asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

fn view(skill: Skill, with_body: bool) -> SkillView {
    let usage = skill.sidecar;
    SkillView {
        name: skill.name,
        description: skill.description,
        version: skill.version,
        status: usage.status,
        provenance: usage.provenance,
        pinned: usage.pinned,
        created_at: usage.created_at,
        last_used_at: usage.last_used_at,
        last_viewed_at: usage.last_viewed_at,
        last_patched_at: usage.last_patched_at,
        usage_count: usage.usage_count,
        view_count: usage.view_count,
        patch_count: usage.patch_count,
        body: with_body.then_some(skill.body),
    }
}

fn store(state: &AppState) -> Result<&Arc<SkillStore>, Failure> {
    state.skills.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no skill store".to_string(),
    ))
}

fn failure(e: Error) -> Failure {
    let status = match e {
        Error::NotFound { .. } | Error::Absorbed { .. } => StatusCode::NOT_FOUND,
        Error::Pinned { .. } => StatusCode::CONFLICT,
        Error::InvalidName { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    (status, e.to_string())
}

/// `GET /skills` — every skill, archived ones too, by name.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<SkillView>>, Failure> {
    Ok(Json(
        store(&state)?
            .all()
            .into_iter()
            .map(|s| view(s, false))
            .collect(),
    ))
}

/// `GET /skills/{name}` — one skill, with its body. Reading it here is
/// not Niles using it, so it does not count as a view.
pub async fn get(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<SkillView>, Failure> {
    let skill = store(&state)?.load(&name).map_err(failure)?;
    Ok(Json(view(skill, true)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillChange {
    pub pinned: Option<bool>,
    pub status: Option<SkillStatus>,
}

/// `PATCH /skills/{name}` — pin or unpin it, archive or restore it.
pub async fn change(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(change): Json<SkillChange>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    store.load(&name).map_err(failure)?;
    if let Some(pinned) = change.pinned {
        store.set_pinned(&name, pinned).map_err(failure)?;
    }
    if let Some(status) = change.status {
        store.set_status(&name, status).map_err(failure)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /skills/{name}` — refused while it is pinned.
pub async fn remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, Failure> {
    store(&state)?.delete(&name, None).map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use crate::state::AppState;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use niles_core::{DeviceRegistry, EventBus};
    use niles_skills::{Provenance, SkillStatus, SkillStore};
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

    fn app(store: Option<Arc<SkillStore>>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_skills(store),
        )
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn house() -> Arc<SkillStore> {
        let store = Arc::new(SkillStore::new(100_000));
        store
            .create(
                "evening-wind-down",
                "Dim the house for the evening",
                "0.1.0",
                "1. Dim the living room to 30%.",
                Provenance::AgentCreated,
            )
            .unwrap();
        store
            .create(
                "away",
                "Leaving the house",
                "0.1.0",
                "Turn everything off.",
                Provenance::UserCreated,
            )
            .unwrap();
        store.set_status("away", SkillStatus::Archived).unwrap();
        store
    }

    #[tokio::test]
    async fn lists_every_skill_without_bodies() {
        let app = app(Some(house()));
        let (status, body) = send(&app, "GET", "/skills", None).await;
        assert_eq!(status, StatusCode::OK);
        let listed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(listed[0]["name"], "away");
        assert_eq!(listed[0]["status"], "archived");
        assert_eq!(listed[0]["provenance"], "user-created");
        assert_eq!(listed[1]["name"], "evening-wind-down");
        assert_eq!(listed[1]["provenance"], "agent-created");
        assert_eq!(listed[1]["pinned"], false);
        assert_eq!(listed[1]["usage_count"], 0);
        assert!(listed[1].get("body").is_none());
    }

    #[tokio::test]
    async fn one_skill_comes_with_its_body_and_is_not_counted_as_viewed() {
        let store = house();
        let app = app(Some(store.clone()));
        let (status, body) = send(&app, "GET", "/skills/evening-wind-down", None).await;
        assert_eq!(status, StatusCode::OK);
        let skill: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(skill["body"], "1. Dim the living room to 30%.");
        assert_eq!(
            store.load("evening-wind-down").unwrap().sidecar.view_count,
            0
        );

        let (status, _) = send(&app, "GET", "/skills/nothing", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pins_archives_and_restores() {
        let store = house();
        let app = app(Some(store.clone()));
        let (status, _) = send(
            &app,
            "PATCH",
            "/skills/evening-wind-down",
            Some(json!({ "pinned": true, "status": "archived" })),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let usage = store.load("evening-wind-down").unwrap().sidecar;
        assert!(usage.pinned);
        assert_eq!(usage.status, SkillStatus::Archived);

        send(
            &app,
            "PATCH",
            "/skills/evening-wind-down",
            Some(json!({ "status": "active" })),
        )
        .await;
        let usage = store.load("evening-wind-down").unwrap().sidecar;
        assert!(usage.pinned);
        assert_eq!(usage.status, SkillStatus::Active);
    }

    #[tokio::test]
    async fn a_pinned_skill_is_not_deleted() {
        let store = house();
        store.set_pinned("evening-wind-down", true).unwrap();
        let app = app(Some(store.clone()));
        let (status, reason) = send(&app, "DELETE", "/skills/evening-wind-down", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(reason.contains("pinned"), "{reason}");

        let (status, _) = send(&app, "DELETE", "/skills/away", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(store.list(), vec!["evening-wind-down"]);
    }

    #[tokio::test]
    async fn without_a_store_it_says_so() {
        let (status, reason) = send(&app(None), "GET", "/skills", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(reason.contains("no skill store"));
    }
}

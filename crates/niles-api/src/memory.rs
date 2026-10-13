//! What Niles remembers, from the app.
//!
//! The same store the voice tools and the background review write to.
//! The review writes without anybody watching, so this is where the
//! household sees what got kept and deletes what should not have been.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_memory::{Error, MemoryStore, Target};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type Failure = (StatusCode, String);

#[derive(Debug, Serialize)]
pub struct MemoryView {
    pub user: Vec<String>,
    pub agent: Vec<String>,
}

fn store(state: &AppState) -> Result<&Arc<MemoryStore>, Failure> {
    state.memory.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no memory store".to_string(),
    ))
}

fn target(name: &str) -> Result<Target, Failure> {
    match name {
        "user" => Ok(Target::User),
        "agent" => Ok(Target::Memory),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("no memory called {other:?}; it is \"user\" or \"agent\""),
        )),
    }
}

fn failure(e: Error) -> Failure {
    match e {
        Error::NotFound { .. } => (StatusCode::NOT_FOUND, e.to_string()),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
    }
}

fn texts(store: &MemoryStore, target: Target) -> Vec<String> {
    store.load(target).into_iter().map(|e| e.text).collect()
}

/// `GET /memory` — both documents, entry by entry, in the order kept.
pub async fn list(State(state): State<AppState>) -> Result<Json<MemoryView>, Failure> {
    let store = store(&state)?;
    Ok(Json(MemoryView {
        user: texts(store, Target::User),
        agent: texts(store, Target::Memory),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forget {
    pub text: String,
}

/// `DELETE /memory/{target}` — the entry whose text is exactly `text`.
pub async fn remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(forget): Json<Forget>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    let target = target(&name)?;
    store.forget(target, &forget.text).map_err(failure)?;
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
    use niles_memory::{MemoryConfig, MemoryStore, Target};
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

    fn app(store: Option<Arc<MemoryStore>>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_memory(store),
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

    fn house() -> Arc<MemoryStore> {
        let store = Arc::new(MemoryStore::new(MemoryConfig::default()));
        store
            .add(Target::User, "Mark takes his coffee black")
            .unwrap();
        store
            .add(Target::User, "Mark takes his coffee black on weekdays")
            .unwrap();
        store
            .add(Target::Memory, "The hallway lamp is slow to respond")
            .unwrap();
        store
    }

    fn texts(store: &MemoryStore, target: Target) -> Vec<String> {
        store.load(target).into_iter().map(|e| e.text).collect()
    }

    #[tokio::test]
    async fn lists_both_documents() {
        let (status, body) = send(&app(Some(house())), "GET", "/memory", None).await;
        assert_eq!(status, StatusCode::OK);
        let listed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            listed,
            json!({
                "user": [
                    "Mark takes his coffee black",
                    "Mark takes his coffee black on weekdays",
                ],
                "agent": ["The hallway lamp is slow to respond"],
            })
        );
    }

    #[tokio::test]
    async fn deletes_the_exact_entry_even_when_another_contains_it() {
        let store = house();
        let app = app(Some(store.clone()));
        let (status, reason) = send(
            &app,
            "DELETE",
            "/memory/user",
            Some(json!({ "text": "Mark takes his coffee black on weekdays" })),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{reason}");
        assert_eq!(
            texts(&store, Target::User),
            vec!["Mark takes his coffee black"]
        );
        assert_eq!(texts(&store, Target::Memory).len(), 1);
    }

    #[tokio::test]
    async fn an_entry_inside_a_longer_one_is_deleted_on_its_own() {
        let store = house();
        let app = app(Some(store.clone()));
        let (status, reason) = send(
            &app,
            "DELETE",
            "/memory/user",
            Some(json!({ "text": "Mark takes his coffee black" })),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{reason}");
        assert_eq!(
            texts(&store, Target::User),
            vec!["Mark takes his coffee black on weekdays"]
        );
    }

    #[tokio::test]
    async fn a_near_match_is_not_found() {
        let store = house();
        let app = app(Some(store.clone()));
        let (status, _) = send(
            &app,
            "DELETE",
            "/memory/agent",
            Some(json!({ "text": "hallway lamp is slow" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(texts(&store, Target::Memory).len(), 1);

        let (status, _) = send(
            &app,
            "DELETE",
            "/memory/agent",
            Some(json!({ "text": "Mark takes his coffee black on weekdays" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(texts(&store, Target::User).len(), 2);
    }

    #[tokio::test]
    async fn an_unknown_target_is_a_bad_request() {
        let (status, reason) = send(
            &app(Some(house())),
            "DELETE",
            "/memory/household",
            Some(json!({ "text": "Mark takes his coffee black" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(reason.contains("household"), "{reason}");
    }

    #[tokio::test]
    async fn without_a_store_it_says_so() {
        let app = app(None);
        let (status, reason) = send(&app, "GET", "/memory", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(reason.contains("no memory store"));

        let (status, _) = send(
            &app,
            "DELETE",
            "/memory/user",
            Some(json!({ "text": "anything" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

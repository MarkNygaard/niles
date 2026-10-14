//! Third-party plugins: what is installed and how each is doing, and
//! their pages and widgets.
//!
//! ```text
//! GET  /plugins                                              -> [PluginSummary]
//! GET  /plugins/{id}/surfaces/{surface}?room=kitchen         -> the node tree
//! POST /plugins/{id}/surfaces/{surface}/actions/{control}    -> {toast, rerender}
//! ```
//!
//! A page's tree is the shape `niles_plugins::Surface` documents. A
//! surface the plugin could not draw answers 422 with the plugin's own
//! sentence ("No prices yet."), one whose plugin is down 503 with its
//! status line, and the app shows either in place of the surface.
//!
//! The signed-in person reaches the plugin as their slug, never their
//! address: the speaker name their account is paired with, when it is.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

/// One installed plugin, for the Integrations page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `starting`, `running`, `not_set_up`, `failing` or `stopped`.
    pub status: String,
    /// The card's line: "Working", "Not working: timed out in on-timer
    /// after 10 s".
    pub status_line: String,
    /// The last thing that went wrong, if anything has.
    pub error: Option<String>,
    /// Events dropped because the plugin was not keeping up with them.
    pub dropped_events: u64,
}

/// Why a surface or an action got no answer.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceFailure {
    /// No such plugin or surface.
    NotFound(String),
    /// The request itself is wrong: an action value that is not one.
    BadRequest(String),
    /// The plugin's own error, or a tree Niles would not draw.
    Refused(String),
    /// The plugin is not running.
    Unavailable(String),
}

impl From<SurfaceFailure> for Failure {
    fn from(failure: SurfaceFailure) -> Self {
        match failure {
            SurfaceFailure::NotFound(s) => (StatusCode::NOT_FOUND, s),
            SurfaceFailure::BadRequest(s) => (StatusCode::BAD_REQUEST, s),
            SurfaceFailure::Refused(s) => (StatusCode::UNPROCESSABLE_ENTITY, s),
            SurfaceFailure::Unavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s),
        }
    }
}

/// Where a surface is drawn and for whom.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Viewer {
    /// The room whose card a widget is on.
    pub room: Option<String>,
    /// The signed-in person's slug.
    pub person: Option<String>,
}

/// What answers for the plugins. Implemented by the binary, which runs
/// them; JSON passes through so this crate needs nothing of theirs.
#[async_trait]
pub trait Plugins: Send + Sync {
    fn list(&self) -> Vec<PluginSummary>;
    async fn render(
        &self,
        id: &str,
        surface: &str,
        viewer: Viewer,
    ) -> Result<serde_json::Value, SurfaceFailure>;
    async fn act(
        &self,
        id: &str,
        surface: &str,
        control: &str,
        value: serde_json::Value,
        viewer: Viewer,
    ) -> Result<serde_json::Value, SurfaceFailure>;
}

fn plugins(state: &AppState) -> Result<&dyn Plugins, Failure> {
    state.plugins.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles runs no plugins".to_string(),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct SurfaceQuery {
    #[serde(default)]
    pub room: Option<String>,
}

fn viewer(state: &AppState, headers: &HeaderMap, query: SurfaceQuery) -> Viewer {
    let person = state.config.as_ref().and_then(|store| {
        let config = store.current();
        let email = crate::auth::signed_in_as(headers, &config)?;
        config.auth.person_for(&email)?.speaker.clone()
    });
    Viewer {
        room: query.room.filter(|r| !r.trim().is_empty()),
        person,
    }
}

/// `GET /plugins`
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<PluginSummary>>, Failure> {
    Ok(Json(plugins(&state)?.list()))
}

/// `GET /plugins/{id}/surfaces/{surface}?room=kitchen`
pub async fn render(
    State(state): State<AppState>,
    Path((id, surface)): Path<(String, String)>,
    Query(query): Query<SurfaceQuery>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Failure> {
    let viewer = viewer(&state, &headers, query);
    Ok(Json(plugins(&state)?.render(&id, &surface, viewer).await?))
}

/// `POST /plugins/{id}/surfaces/{surface}/actions/{control}`, with the
/// value: `{"type":"pressed"}`, `{"type":"toggled","value":true}`,
/// `{"type":"number","value":21.5}`, `{"type":"chosen","value":"DK2"}`
/// or `{"type":"text","value":"hello"}`.
pub async fn act(
    State(state): State<AppState>,
    Path((id, surface, control)): Path<(String, String, String)>,
    Query(query): Query<SurfaceQuery>,
    headers: HeaderMap,
    Json(value): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, Failure> {
    let viewer = viewer(&state, &headers, query);
    Ok(Json(
        plugins(&state)?
            .act(&id, &surface, &control, value, viewer)
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _: String, _: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Fake {
        seen: Mutex<Vec<(String, Viewer)>>,
    }

    #[async_trait]
    impl Plugins for Fake {
        fn list(&self) -> Vec<PluginSummary> {
            vec![PluginSummary {
                id: "spotprice".into(),
                name: "Spot prices".into(),
                version: "0.1.0".into(),
                status: "failing".into(),
                status_line: "Not working: timed out in on-timer after 10 s".into(),
                error: Some("timed out in on-timer after 10 s".into()),
                dropped_events: 0,
            }]
        }

        async fn render(
            &self,
            id: &str,
            surface: &str,
            viewer: Viewer,
        ) -> Result<serde_json::Value, SurfaceFailure> {
            self.seen.lock().unwrap().push((surface.into(), viewer));
            match (id, surface) {
                ("spotprice", "electricity") => {
                    Ok(serde_json::json!({"nodes": [{"type": "stack", "children": []}]}))
                }
                ("spotprice", "empty") => Err(SurfaceFailure::Refused("No prices yet.".into())),
                _ => Err(SurfaceFailure::NotFound("no such page".into())),
            }
        }

        async fn act(
            &self,
            _: &str,
            _: &str,
            control: &str,
            value: serde_json::Value,
            _: Viewer,
        ) -> Result<serde_json::Value, SurfaceFailure> {
            match (control, value["type"].as_str()) {
                ("refresh", Some("pressed")) => {
                    Ok(serde_json::json!({"toast": "Prices updated", "rerender": true}))
                }
                _ => Err(SurfaceFailure::BadRequest("not an action value".into())),
            }
        }
    }

    fn app(fake: Arc<Fake>) -> axum::Router {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher),
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_plugins(Some(fake));
        crate::router(state)
    }

    async fn send(app: axum::Router, request: Request<Body>) -> (StatusCode, String) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn get(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn the_list_carries_the_status_and_the_last_error() {
        let (status, body) = send(app(Arc::default()), get("/plugins")).await;
        assert_eq!(status, StatusCode::OK);
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["id"], "spotprice");
        assert_eq!(list[0]["status"], "failing");
        assert_eq!(list[0]["error"], "timed out in on-timer after 10 s");
    }

    #[tokio::test]
    async fn a_surface_is_drawn_for_its_room() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            get("/plugins/spotprice/surfaces/electricity?room=kitchen"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"stack\""));
        let seen = fake.seen.lock().unwrap();
        assert_eq!(seen[0].1.room.as_deref(), Some("kitchen"));
        assert_eq!(seen[0].1.person, None, "nobody is signed in");
    }

    #[tokio::test]
    async fn the_plugins_own_error_and_a_missing_page_are_told_apart() {
        let (status, body) = send(
            app(Arc::default()),
            get("/plugins/spotprice/surfaces/empty"),
        )
        .await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "No prices yet.")
        );
        let (status, _) = send(app(Arc::default()), get("/plugins/nope/surfaces/x")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_control_is_used() {
        let request = Request::post("/plugins/spotprice/surfaces/electricity/actions/refresh")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"type":"pressed"}"#))
            .unwrap();
        let (status, body) = send(app(Arc::default()), request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("Prices updated"));
    }

    #[tokio::test]
    async fn without_plugins_the_routes_say_so() {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher),
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        );
        let (status, _) = send(crate::router(state), get("/plugins")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

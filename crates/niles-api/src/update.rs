//! About Niles: which build runs, and whether a newer one is published.
//!
//! `GET /system/update` answers what the last check kept and never
//! waits on the network, so About Niles opens at once; the background
//! loop, opening the page and `update_checked` keep it current.
//! `POST /system/update/check` checks now. Both degrade to a plain
//! answer when the binary started without the update check.

use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

/// `GET /system/update` — the kept answer: which build runs, whether a
/// newer one is published, and what changed between them.
pub async fn status(State(state): State<AppState>) -> Response {
    let Some(updates) = &state.updates else {
        return unsupported();
    };
    Json(updates.status()).into_response()
}

/// `POST /system/update/check` — ask now. Single-flight and throttled
/// in `niles-update`, so a page opened twice can't hammer GitHub.
pub async fn check(State(state): State<AppState>) -> Response {
    let Some(updates) = &state.updates else {
        return unsupported();
    };
    Json(updates.check().await).into_response()
}

/// Subcommands that don't build the update check still serve the API,
/// so this is a missing capability rather than an error.
fn unsupported() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "this Niles was started without the update check, so it can't say \
                      whether a newer one is published"
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use niles_core::EventBus;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// The router with the update check over a stand-in, as `niles
    /// serve` would wire it.
    fn fixture(updates: Option<Arc<niles_update::Updates>>) -> axum::Router {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(crate::config_tests::NoopPublisher)
                as Arc<dyn crate::publish::DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            EventBus::default(),
        )
        .with_config_store(Some(Arc::new(
            niles_config::ConfigStore::from_str_in_memory(&crate::config_tests::base_toml())
                .expect("fixture is valid"),
        )))
        .with_updates(updates);
        crate::server::router(state)
    }

    /// A dev build's check over a stand-in, with nothing listening on
    /// the GitHub side.
    fn dev_updates(bus: EventBus) -> Arc<niles_update::Updates> {
        Arc::new(niles_update::Updates::at(
            niles_update::Build::new("0.6.0", None, None),
            bus,
            niles_update::Registry::at("http://127.0.0.1:9").expect("the address parses"),
            niles_update::GitHub::at("http://127.0.0.1:9").expect("the address parses"),
        ))
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    fn post(uri: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn a_dev_build_says_so() {
        let app = fixture(Some(dev_updates(EventBus::default())));
        let (status, body) = send(&app, get("/system/update")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "dev_build");
        assert_eq!(body["running"]["version"], "0.6.0");
        assert_eq!(body["running"]["commit"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn a_failed_check_answers_not_checked_with_the_reason() {
        // A port that was listening a moment ago and isn't now.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds an ephemeral port");
        let address = format!("http://{}", listener.local_addr().expect("the address"));
        drop(listener);
        let build = niles_update::Build::new(
            "0.6.0",
            Some("0123456789abcdef0123456789abcdef01234567"),
            Some("2026-11-01T00:00:00Z"),
        );
        let updates = Arc::new(niles_update::Updates::at(
            build,
            EventBus::default(),
            niles_update::Registry::at(&address).expect("the address parses"),
            niles_update::GitHub::at(&address).expect("the address parses"),
        ));
        let app = fixture(Some(updates));
        let (status, body) = send(&app, post("/system/update/check")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "not_checked");
        assert!(
            !body["failed"]["reason"]
                .as_str()
                .expect("a reason")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn without_the_check_the_routes_say_so() {
        let app = fixture(None);
        let (status, body) = send(&app, get("/system/update")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body["error"].as_str().expect("an error").contains("update"));
    }
}

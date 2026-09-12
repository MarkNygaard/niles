//! HTTP-level tests for the `/config` routes.
//!
//! Kept in their own file because they need a `ConfigStore` fixture that
//! the device-API tests have no use for.

use crate::publish::DevicePublisher;
use crate::server::router;
use crate::state::AppState;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use niles_config::ConfigStore;
use niles_core::{DeviceRegistry, EventBus};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

#[derive(Clone)]
struct NoopPublisher;

#[async_trait]
impl DevicePublisher for NoopPublisher {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

fn base_toml() -> String {
    // A minimal config `validate()` accepts. Kept local rather than
    // reaching into niles-config's test fixtures, which are crate-private.
    r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[mqtt]
host = "192.168.42.16"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

[wyoming]
bind_address = "0.0.0.0:10300"

[stt]
api_key_env = "GROQ_API_KEY"

[tts]

[llm]
api_key_env = "GROQ_API_KEY"

[lighting]
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "12:00"
kelvin = 4500

[[lighting.color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#
    .to_string()
}

fn app(with_store: bool) -> axum::Router {
    let store = with_store.then(|| {
        Arc::new(ConfigStore::from_str_in_memory(&base_toml()).expect("fixture is valid"))
    });
    let state = AppState::new(
        Arc::new(DeviceRegistry::new()),
        Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
        Arc::new("zigbee2mqtt".into()),
        EventBus::default(),
    )
    .with_config_store(store);
    router(state)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn patch(body: Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri("/config")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn get_config_returns_effective_values_and_reload_info() {
    let app = app(true);
    let (status, body) = send(&app, get("/config")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["effective"]["lighting"]["daytime_brightness"], 100);
    assert_eq!(body["overrides"], json!({}));

    let sections = body["sections"].as_array().unwrap();
    let lighting = sections
        .iter()
        .find(|s| s["name"] == "lighting")
        .expect("lighting section listed");
    assert_eq!(lighting["reload"], "hot");
    assert_eq!(lighting["overridden"], false);
    let mqtt = sections.iter().find(|s| s["name"] == "mqtt").unwrap();
    assert_eq!(mqtt["reload"], "boot");
}

#[tokio::test]
async fn patch_changes_a_value_and_reports_the_diff() {
    let app = app(true);
    let (status, body) = send(&app, patch(json!({"lighting": {"daytime_brightness": 85}}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["revision"], 1);
    assert_eq!(body["noop"], false);
    assert_eq!(body["summary"], "lighting.daytime_brightness 100 → 85");
    assert_eq!(body["changes"][0]["path"], "lighting.daytime_brightness");
    assert_eq!(body["changes"][0]["from"], 100);
    assert_eq!(body["changes"][0]["to"], 85);

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["effective"]["lighting"]["daytime_brightness"], 85);
    assert_eq!(after["overrides"]["lighting"]["daytime_brightness"], 85);
}

#[tokio::test]
async fn patch_leaves_unmentioned_values_alone() {
    let app = app(true);
    send(&app, patch(json!({"lighting": {"daytime_brightness": 85}}))).await;
    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["effective"]["lighting"]["night_floor_brightness"], 15);
    assert_eq!(after["effective"]["home"]["name"], "test home");
}

#[tokio::test]
async fn an_invalid_value_is_rejected_with_the_reason() {
    let app = app(true);
    let (status, body) = send(&app, patch(json!({"lighting": {"morning_start": "09:00"}}))).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a bad value is the caller's mistake, not a server fault"
    );
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("morning_start"), "says which field: {error}");

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["overrides"], json!({}), "nothing was stored");
}

#[tokio::test]
async fn a_misspelled_key_is_rejected() {
    let app = app(true);
    let (status, body) = send(&app, patch(json!({"lighting": {"daytime_brightnes": 85}}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("daytime_brightnes")
    );
}

#[tokio::test]
async fn a_non_object_body_is_a_bad_request() {
    let app = app(true);
    let (status, _) = send(&app, patch(json!([1, 2, 3]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setting_a_value_to_what_it_already_is_reports_a_noop() {
    let app = app(true);
    let (status, body) = send(
        &app,
        patch(json!({"lighting": {"daytime_brightness": 100}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["noop"], true);
    assert_eq!(body["revision"], 0);
}

#[tokio::test]
async fn changing_a_boot_section_says_a_restart_is_needed() {
    let app = app(true);
    let (status, body) = send(
        &app,
        patch(json!({"api": {"bind_address": "0.0.0.0:9090"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the write is accepted");
    assert_eq!(
        body["needs_restart"],
        json!(["api"]),
        "but the caller is told it won't take effect yet"
    );
}

#[tokio::test]
async fn an_optional_value_the_base_file_omits_can_still_be_set() {
    // The UI offers every known setting, configured or not. A value that
    // has never been set is the normal case for an optional one — and is
    // exactly the case that used to have no way in.
    let app = app(true);
    let (status, body) = send(&app, patch(json!({"lighting": {"ambient_brightness": 25}}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["changes"][0]["from"], Value::Null, "nothing was there");
    assert_eq!(body["changes"][0]["to"], 25);

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["effective"]["lighting"]["ambient_brightness"], 25);
}

#[tokio::test]
async fn a_section_the_base_file_omits_can_still_be_set() {
    let app = app(true);
    let (status, body) = send(
        &app,
        patch(json!({"ambient_lights": {"devices": ["living_room/tv_lightstrip"]}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["needs_restart"],
        json!(["ambient_lights"]),
        "which lights are ambient is read once, at startup"
    );

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(
        after["effective"]["ambient_lights"]["devices"],
        json!(["living_room/tv_lightstrip"])
    );
}

#[tokio::test]
async fn delete_returns_a_value_to_the_base() {
    let app = app(true);
    send(&app, patch(json!({"lighting": {"daytime_brightness": 85}}))).await;
    let req = Request::builder()
        .method("DELETE")
        .uri("/config/lighting.daytime_brightness")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["effective"]["lighting"]["daytime_brightness"], 100);
    assert_eq!(after["overrides"], json!({}));
}

#[tokio::test]
async fn history_records_each_change_with_its_source() {
    let app = app(true);
    send(&app, patch(json!({"lighting": {"daytime_brightness": 85}}))).await;
    let (status, body) = send(&app, get("/config/history")).await;
    assert_eq!(status, StatusCode::OK);
    let history = body.as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["id"], 1);
    assert_eq!(history[0]["source"], "api");
    assert_eq!(
        history[0]["summary"],
        "lighting.daytime_brightness 100 → 85"
    );
}

#[tokio::test]
async fn undo_walks_back_the_last_change() {
    let app = app(true);
    send(&app, patch(json!({"lighting": {"daytime_brightness": 85}}))).await;
    let undo = Request::builder()
        .method("POST")
        .uri("/config/undo")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, undo).await;
    assert_eq!(status, StatusCode::OK);

    let (_, after) = send(&app, get("/config")).await;
    assert_eq!(after["effective"]["lighting"]["daytime_brightness"], 100);
}

#[tokio::test]
async fn undo_with_nothing_to_undo_is_not_a_success() {
    let app = app(true);
    let undo = Request::builder()
        .method("POST")
        .uri("/config/undo")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, undo).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_routes_report_when_no_store_was_configured() {
    // `niles lighting` and friends serve the device API without a store.
    let app = app(false);
    let (status, _) = send(&app, get("/config")).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn the_device_api_still_works_without_a_store() {
    let app = app(false);
    let (status, _) = send(&app, get("/healthz")).await;
    assert_eq!(status, StatusCode::OK);
}

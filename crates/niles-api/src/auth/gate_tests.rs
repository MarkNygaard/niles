//! The gate, against a real router.
//!
//! The unit tests next door check the pieces; these check the thing
//! everybody actually relies on — that a request without a session
//! does not reach `/devices`, and that one with a session that has
//! since been revoked does not either.

use crate::auth::session::{self, Session};
use crate::publish::DevicePublisher;
use crate::server::router;
use crate::state::AppState;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use niles_config::ConfigStore;
use niles_core::{DeviceRegistry, EventBus};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET_VAR: &str = "NILES_TEST_AUTH_SESSION_SECRET";
const SECRET: &str = "a-key-that-outlives-a-restart";
const CLIENT_ID_VAR: &str = "NILES_TEST_AUTH_CLIENT_ID";
const CLIENT_SECRET_VAR: &str = "NILES_TEST_AUTH_CLIENT_SECRET";

#[derive(Clone)]
struct NoopPublisher;

#[async_trait]
impl DevicePublisher for NoopPublisher {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

fn set_env() {
    // SAFETY: these are test-only variable names nothing else reads.
    unsafe {
        std::env::set_var(SECRET_VAR, SECRET);
        std::env::set_var(CLIENT_ID_VAR, "client-id");
        std::env::set_var(CLIENT_SECRET_VAR, "client-secret");
    }
}

/// The base fixture plus an `[auth]` section listing `allowed`.
fn app_allowing(allowed: &str, api_token: Option<&str>) -> axum::Router {
    set_env();
    let toml = format!(
        "{}\n[auth]\ngithub_client_id_env = \"{CLIENT_ID_VAR}\"\n\
         github_client_secret_env = \"{CLIENT_SECRET_VAR}\"\n\
         session_secret_env = \"{SECRET_VAR}\"\n{allowed}\n",
        crate::config_tests::base_toml()
    );
    let store = Arc::new(ConfigStore::from_str_in_memory(&toml).expect("fixture is valid"));
    let state = AppState::new(
        Arc::new(DeviceRegistry::new()),
        Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
        Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
        EventBus::default(),
    )
    .with_config_store(Some(store))
    .with_api_token(api_token.map(str::to_string));
    router(state)
}

/// `[auth]` naming variables that are not set — a platform that dropped
/// the keys on the way in.
fn app_with_unreadable_secrets() -> axum::Router {
    let toml = format!(
        "{}
[auth]
github_client_id_env = \"NILES_TEST_AUTH_NEVER_SET_ID\"
         github_client_secret_env = \"NILES_TEST_AUTH_NEVER_SET_SECRET\"
         session_secret_env = \"NILES_TEST_AUTH_NEVER_SET_SESSION\"
allowed = []
",
        crate::config_tests::base_toml()
    );
    let store = Arc::new(ConfigStore::from_str_in_memory(&toml).expect("fixture is valid"));
    let state = AppState::new(
        Arc::new(DeviceRegistry::new()),
        Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
        Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
        EventBus::default(),
    )
    .with_config_store(Some(store));
    router(state)
}

fn signed_in_as(email: &str) -> String {
    let token = session::sign(SECRET, &Session::new(email));
    format!("{}={token}", session::COOKIE)
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn get_with(uri: &str, name: header::HeaderName, value: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(name, value)
        .body(Body::empty())
        .unwrap()
}

const ONE_PERSON: &str = r#"allowed = [{ email = "mark@example.com" }]"#;

#[tokio::test]
async fn nobody_listed_means_everything_is_open() {
    // A fresh install has to be usable, or there is no way to add the
    // first person. This is the bootstrap, and it is deliberate.
    let app = app_allowing("allowed = []", None);
    assert_eq!(send(&app, get("/devices")).await.0, StatusCode::OK);
}

#[tokio::test]
async fn with_somebody_listed_a_stranger_is_refused() {
    let app = app_allowing(ONE_PERSON, None);
    assert_eq!(
        send(&app, get("/devices")).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_session_for_an_allowed_address_gets_in() {
    let app = app_allowing(ONE_PERSON, None);
    let request = get_with(
        "/devices",
        header::COOKIE,
        &signed_in_as("mark@example.com"),
    );
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
}

#[tokio::test]
async fn taking_somebody_off_the_list_revokes_the_cookie_they_already_hold() {
    // The reason the cookie carries the address rather than a session
    // id: the list is read now, not when the cookie was signed.
    let app = app_allowing(ONE_PERSON, None);
    let request = get_with(
        "/devices",
        header::COOKIE,
        &signed_in_as("majse@example.com"),
    );
    assert_eq!(
        send(&app, request).await.0,
        StatusCode::UNAUTHORIZED,
        "a validly-signed cookie for somebody not on the list is nobody"
    );
}

#[tokio::test]
async fn a_cookie_signed_with_another_key_is_refused() {
    let app = app_allowing(ONE_PERSON, None);
    let forged = session::sign("not-our-key", &Session::new("mark@example.com"));
    let request = get_with(
        "/devices",
        header::COOKIE,
        &format!("{}={forged}", session::COOKIE),
    );
    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_operator_token_gets_in_without_being_a_person() {
    // This is how `/logs` stays readable from a terminal, and the way
    // back in if the list is ever emptied.
    let app = app_allowing(ONE_PERSON, Some("operator-token"));
    let request = get_with("/devices", header::AUTHORIZATION, "Bearer operator-token");
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_wrong_token_is_still_refused() {
    let app = app_allowing(ONE_PERSON, Some("operator-token"));
    let request = get_with("/devices", header::AUTHORIZATION, "Bearer not-the-token");
    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_empty_token_setting_is_not_a_password_of_nothing() {
    // A placeholder nobody filled in must not authenticate anybody.
    let app = app_allowing(ONE_PERSON, Some("   "));
    let request = get_with("/devices", header::AUTHORIZATION, "Bearer ");
    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_liveness_probe_never_needs_a_session() {
    let app = app_allowing(ONE_PERSON, None);
    assert_eq!(send(&app, get("/healthz")).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_page_is_still_served_so_it_can_show_a_sign_in_screen() {
    // Redirecting a navigation would mean the UI never gets to ask who
    // it is talking to.
    let app = app_allowing(ONE_PERSON, None);
    let request = get_with("/", header::ACCEPT, "text/html");
    let status = app.oneshot(request).await.unwrap().status();
    assert_ne!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn status_says_whether_to_offer_a_sign_in_button() {
    let app = app_allowing(ONE_PERSON, None);
    let (status, body) = send(&app, get("/auth/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["signed_in_as"], Value::Null);
}

#[tokio::test]
async fn status_names_who_is_signed_in() {
    let app = app_allowing(ONE_PERSON, None);
    let request = get_with(
        "/auth/status",
        header::COOKIE,
        &signed_in_as("mark@example.com"),
    );
    let (_, body) = send(&app, request).await;
    assert_eq!(body["signed_in_as"], "mark@example.com");
}

#[tokio::test]
async fn status_does_not_name_somebody_who_has_been_removed() {
    let app = app_allowing(ONE_PERSON, None);
    let request = get_with(
        "/auth/status",
        header::COOKIE,
        &signed_in_as("majse@example.com"),
    );
    let (_, body) = send(&app, request).await;
    assert_eq!(body["signed_in_as"], Value::Null);
}

#[tokio::test]
async fn the_config_routes_are_behind_the_gate_too() {
    // The allowlist itself lives there, so leaving it open would mean
    // anybody could add themselves.
    let app = app_allowing(ONE_PERSON, None);
    for path in ["/config", "/config/history", "/logs"] {
        assert_eq!(
            send(&app, get(path)).await.0,
            StatusCode::UNAUTHORIZED,
            "{path} must be behind the gate"
        );
    }
}

// Only meaningful with the bundle embedded: without the `ui`
// feature there are no assets, and nothing extra is public.
#[cfg(feature = "ui")]
#[tokio::test]
async fn the_pages_own_assets_are_served_to_somebody_not_signed_in_yet() {
    // The shell is useless without its script, and a browser asks for
    // that with `Accept: */*` — so gating on "does it want HTML" alone
    // serves the page and then refuses the JavaScript that would have
    // drawn the sign-in screen on it.
    let app = app_allowing(ONE_PERSON, None);
    let (status, _) = send(&app, get("/manifest.webmanifest")).await;
    assert_eq!(status, StatusCode::OK, "the app must be able to boot");
}

// Only meaningful with the bundle embedded: without the `ui`
// feature there are no assets, and nothing extra is public.
#[cfg(feature = "ui")]
#[tokio::test]
async fn a_ui_asset_is_public_but_an_api_route_of_the_same_shape_is_not() {
    // The rule is "is this a file the bundle ships", not "does the path
    // look static" — so a route added later is still behind the gate.
    let app = app_allowing(ONE_PERSON, None);
    assert_eq!(send(&app, get("/icon-192.png")).await.0, StatusCode::OK);
    assert_eq!(
        send(&app, get("/assets/there-is-no-such-file.js")).await.0,
        StatusCode::UNAUTHORIZED,
        "a path that merely looks like an asset is not one"
    );
}

#[tokio::test]
async fn every_route_this_api_serves_is_either_named_public_or_refused() {
    // The check somebody should be able to read before exposing this to
    // the internet: the list of what answers without a session, in one
    // place, with everything else proven closed.
    let app = app_allowing(ONE_PERSON, None);

    // Public, each for a stated reason.
    for (path, why) in [
        ("/healthz", "a liveness probe cannot hold a session"),
        (
            "/auth/status",
            "asking whether to sign in cannot require it",
        ),
    ] {
        assert_eq!(
            send(&app, get(path)).await.0,
            StatusCode::OK,
            "{path}: {why}"
        );
    }

    // Everything that carries or changes state.
    for path in [
        "/devices",
        "/rooms/kitchen",
        "/config",
        "/config/history",
        "/logs",
        "/events/stream",
    ] {
        assert_eq!(
            send(&app, get(path)).await.0,
            StatusCode::UNAUTHORIZED,
            "{path} answered without a session"
        );
    }

    // And the writes, which is where it would actually hurt.
    for (method, path) in [
        ("POST", "/rooms/kitchen"),
        ("POST", "/rooms/kitchen/ceiling"),
        ("POST", "/config/undo"),
        ("PATCH", "/config"),
        ("DELETE", "/config/lighting.daytime_brightness"),
    ] {
        let request = Request::builder()
            .uri(path)
            .method(method)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(
            send(&app, request).await.0,
            StatusCode::UNAUTHORIZED,
            "{method} {path} was accepted without a session"
        );
    }
}

#[tokio::test]
async fn status_tells_a_missing_secret_apart_from_an_empty_list() {
    // Both report sign-in as off. Only one of them is open to anybody,
    // and from outside this is the only way to see which.
    let waiting = app_allowing("allowed = []", None);
    let (_, body) = send(&waiting, get("/auth/status")).await;
    assert_eq!(body["enabled"], false);
    assert_eq!(
        body["configured"], true,
        "secrets are readable; this install is merely waiting for its first person"
    );

    let broken = app_with_unreadable_secrets();
    let (_, body) = send(&broken, get("/auth/status")).await;
    assert_eq!(body["enabled"], false);
    assert_eq!(
        body["configured"], false,
        "the secrets never arrived, and saying so is the whole point"
    );
}

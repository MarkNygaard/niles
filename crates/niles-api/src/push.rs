//! Notifications on the signed-in person's devices: the switch and the
//! device list on My profile. How it works is in `niles-webpush`.
//!
//! Every `/me/push` route acts for the caller, by the key their chat is
//! kept under, and never for a person named in the request.

use crate::auth::github::header_str;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use niles_webpush::{Message, NewSubscription, Subscription, WebPush, device_label};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

type Failure = (StatusCode, Json<Value>);

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, Json(json!({ "error": message.into() })))
}

fn push(state: &AppState) -> Result<&Arc<WebPush>, Failure> {
    state.push.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "notifications need Niles's database and its secrets key, and this instance has none",
        )
    })
}

fn storage(e: niles_webpush::Error) -> Failure {
    match e {
        niles_webpush::Error::Invalid { .. } => {
            failed(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
        }
        niles_webpush::Error::Storage(_) => failed(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

#[derive(Debug, Serialize)]
pub struct Key {
    /// The server's public key, base64url, for `applicationServerKey`.
    pub key: String,
}

/// `GET /push/key`
pub async fn key(State(state): State<AppState>) -> Result<Json<Key>, Failure> {
    let key = push(&state)?.public_key().await.map_err(storage)?;
    Ok(Json(Key { key }))
}

#[derive(Debug, Serialize)]
pub struct Devices {
    /// Newest first.
    pub devices: Vec<Subscription>,
}

/// `GET /me/push`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Devices>, Failure> {
    let push = push(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let devices = push.store().subscriptions(&person).await.map_err(storage)?;
    Ok(Json(Devices { devices }))
}

/// `PushSubscription.toJSON()`, as the browser makes it.
#[derive(Debug, Deserialize)]
pub struct Subscribe {
    pub endpoint: String,
    pub keys: Keys,
}

#[derive(Debug, Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

/// `POST /me/push`
pub async fn subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Subscribe>,
) -> Result<StatusCode, Failure> {
    let push = push(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let subscription = NewSubscription {
        endpoint: body.endpoint,
        p256dh: body.keys.p256dh,
        auth: body.keys.auth,
        device: device_label(header_str(&headers, header::USER_AGENT).unwrap_or_default()),
        origin: origin(&headers),
    }
    .checked()
    .map_err(storage)?;
    push.store()
        .subscribe(&person, &subscription)
        .await
        .map_err(storage)?;
    tracing::info!(
        "[push] {person} turned notifications on: {}",
        subscription.device
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct Unsubscribe {
    pub endpoint: String,
}

/// `DELETE /me/push`. Done whether or not it was there: the device
/// asking wants it gone, and gone is what it is.
pub async fn unsubscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Unsubscribe>,
) -> Result<StatusCode, Failure> {
    let push = push(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    push.store()
        .unsubscribe(&person, &body.endpoint)
        .await
        .map_err(storage)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/push/test`: one notification to each of the caller's
/// devices, answering how many push services took it.
pub async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<niles_webpush::Delivery>, Failure> {
    let push = push(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let message = Message {
        title: "Niles".into(),
        body: "This is how Niles tells you something is ready.".into(),
        url: "/#/me/profile".into(),
        tag: "test".into(),
    };
    push.send(&person, &message)
        .await
        .map(Json)
        .map_err(storage)
}

/// The address the app was open at, kept as the contact (`sub`) in the
/// token every push to this device carries.
///
/// There is no configured contact, and asking for one would be a
/// setting nobody understands. Apple refuses a token without a usable
/// `sub`; the house's own https address is one, and it is the address
/// a push service's operator would look at anyway. The browser says it
/// in `Origin`; behind an ingress that drops it, `Host` does, and TLS
/// ends at the ingress, so it is https unless Niles is on this machine.
fn origin(headers: &HeaderMap) -> String {
    if let Some(origin) = header_str(headers, header::ORIGIN).filter(|o| o.contains("://")) {
        return origin.trim().to_string();
    }
    let host = header_str(headers, "x-forwarded-host")
        .or_else(|| header_str(headers, header::HOST))
        .and_then(|h| h.split(',').next())
        .unwrap_or("localhost")
        .trim();
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    if matches!(name, "localhost" | "127.0.0.1" | "[::1]") {
        format!("http://{host}")
    } else {
        format!("https://{host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_webpush::{MemoryPush, PushStore};
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_PUSH_SESSION_SECRET";
    const SECRET: &str = "the-push-page-secret";

    /// RFC 8291's example browser keys: a real point and secret.
    const P256DH: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) \
        AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    struct Fixture {
        app: axum::Router,
        store: Arc<MemoryPush>,
    }

    fn fixture(with_push: bool) -> Fixture {
        // SAFETY: test-only variables nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_PUSH_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_PUSH_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_PUSH_CLIENT_ID"
github_client_secret_env = "NILES_TEST_PUSH_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let store = Arc::new(MemoryPush::default());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_push(
            with_push.then(|| Arc::new(WebPush::new(store.clone(), reqwest::Client::new()))),
        );
        Fixture {
            app: crate::server::router(state),
            store,
        }
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(
                header::COOKIE,
                format!(
                    "{}={}",
                    session::COOKIE,
                    session::sign(SECRET, &Session::new(email, None))
                ),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::HOST, "niles.example")
            .header(header::USER_AGENT, IPHONE)
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn subscription(endpoint: &str) -> Value {
        json!({
            "endpoint": endpoint,
            "expirationTime": null,
            "keys": { "p256dh": P256DH, "auth": AUTH },
        })
    }

    #[tokio::test]
    async fn the_key_is_a_public_key_a_browser_can_subscribe_with() {
        let f = fixture(true);
        let (status, body) = send(&f.app, by("mark@example.com", "GET", "/push/key", None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let key = body["key"].as_str().unwrap();
        // 65 bytes of base64url, unpadded.
        assert_eq!(key.len(), 87, "{key}");
        assert!(!key.contains('=') && !key.contains('+') && !key.contains('/'));
    }

    #[tokio::test]
    async fn a_device_subscribed_is_listed_for_its_person_alone() {
        let f = fixture(true);
        let (status, _) = send(
            &f.app,
            by(
                "mark@example.com",
                "POST",
                "/me/push",
                Some(subscription("https://web.push.apple.com/abc")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (_, mine) = send(&f.app, by("mark@example.com", "GET", "/me/push", None)).await;
        let device = &mine["devices"][0];
        assert_eq!(device["device"], "iPhone");
        assert_eq!(device["endpoint"], "https://web.push.apple.com/abc");
        assert!(device.get("auth").is_none() && device.get("p256dh").is_none());

        let (_, hers) = send(&f.app, by("majse@example.com", "GET", "/me/push", None)).await;
        assert_eq!(hers["devices"], json!([]));

        let kept = f.store.subscriptions("person:mark").await.unwrap();
        assert_eq!(kept[0].origin, "https://niles.example");
    }

    #[tokio::test]
    async fn a_device_is_removed_by_its_owner_and_not_by_anyone_else() {
        let f = fixture(true);
        let endpoint = "https://fcm.googleapis.com/fcm/send/xyz";
        send(
            &f.app,
            by(
                "mark@example.com",
                "POST",
                "/me/push",
                Some(subscription(endpoint)),
            ),
        )
        .await;
        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "DELETE",
                "/me/push",
                Some(json!({ "endpoint": endpoint })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.store.subscriptions("person:mark").await.unwrap().len(), 1);

        send(
            &f.app,
            by(
                "mark@example.com",
                "DELETE",
                "/me/push",
                Some(json!({ "endpoint": endpoint })),
            ),
        )
        .await;
        assert!(
            f.store
                .subscriptions("person:mark")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_subscription_that_cannot_be_pushed_to_is_refused() {
        let f = fixture(true);
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "POST",
                "/me/push",
                Some(subscription("http://push.example/insecure")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    #[tokio::test]
    async fn a_test_with_no_devices_sends_nothing() {
        let f = fixture(true);
        let (status, body) = send(
            &f.app,
            by("mark@example.com", "POST", "/me/push/test", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["sent"], 0);
    }

    #[tokio::test]
    async fn without_a_database_every_route_says_why() {
        let f = fixture(false);
        for (method, uri, body) in [
            ("GET", "/push/key", None),
            ("GET", "/me/push", None),
            (
                "POST",
                "/me/push",
                Some(subscription("https://push.example/x")),
            ),
            (
                "DELETE",
                "/me/push",
                Some(json!({ "endpoint": "https://push.example/x" })),
            ),
            ("POST", "/me/push/test", None),
        ] {
            let (status, answer) = send(&f.app, by("mark@example.com", method, uri, body)).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {uri}");
            assert!(answer["error"].as_str().unwrap().contains("database"));
        }
    }

    #[test]
    fn the_contact_is_the_address_the_app_was_open_at() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://niles.example".parse().unwrap());
        headers.insert(header::HOST, "niles-api.niles.svc:8080".parse().unwrap());
        assert_eq!(origin(&headers), "https://niles.example");

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "niles.example".parse().unwrap());
        assert_eq!(origin(&headers), "https://niles.example");

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-host", "niles.example".parse().unwrap());
        headers.insert(header::HOST, "10.0.0.5:8080".parse().unwrap());
        assert_eq!(origin(&headers), "https://niles.example");

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:5173".parse().unwrap());
        assert_eq!(origin(&headers), "http://localhost:5173");
    }
}

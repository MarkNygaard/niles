//! A person's training data in, from their phone, and the key it comes
//! with. See ARCHITECTURE.md § Training (per person).
//!
//! `POST /training/ingest` is the one route that answers to a training
//! key, and the key answers to nothing else. It is exempt from the
//! sign-in gate because it authenticates itself, the way the GitHub
//! callback does; the key it takes is not the house's API key, so
//! anywhere else the gate refuses it like any other stranger.
//!
//! The key is made, replaced and revoked on the person's own page, by
//! their session: `/me/training`. It is shown once, when made, and kept
//! only as a hash.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_training::{Overview, TrainingStore, hash_key, person_key};
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Failure = (StatusCode, String);

/// What Health Auto Export may send in one request. Its first sync can
/// carry months of workouts with their heart-rate samples and routes.
pub const MAX_INGEST_BYTES: usize = 10 * 1024 * 1024;

/// Posts one person's phone may make in [`WINDOW`]. Generous, because
/// the app's batched first sync arrives as a run of requests; the point
/// is to stop a loop, not to ration a phone.
const POSTS_PER_WINDOW: usize = 120;
const WINDOW: Duration = Duration::from_secs(60);

/// Recent posts per person, for the rate limit.
#[derive(Default)]
pub struct Limits(Mutex<HashMap<String, VecDeque<Instant>>>);

impl Limits {
    fn allow(&self, person: &str, now: Instant) -> bool {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let recent = all.entry(person.to_string()).or_default();
        while recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= POSTS_PER_WINDOW {
            return false;
        }
        recent.push_back(now);
        true
    }
}

fn store(state: &AppState) -> Result<&Arc<dyn TrainingStore>, Failure> {
    state.training.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "training needs Niles's database, and this instance has none".into(),
    ))
}

fn storage(e: niles_training::Error) -> Failure {
    (StatusCode::BAD_GATEWAY, e.to_string())
}

fn refused(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// The person signed in to this browser, keyed as their chat is.
///
/// A session only: the house's API key is nobody in particular, and a
/// training key is somebody's phone, not them.
fn signed_in_person(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let config = state
        .config
        .as_ref()
        .ok_or((
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance has no people configured".to_string(),
        ))?
        .current();
    let email = crate::auth::signed_in_as(headers, &config).ok_or((
        StatusCode::UNAUTHORIZED,
        "sign in to connect your own training data".to_string(),
    ))?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(person_key(speaker.as_deref(), &email))
}

/// `GET /me/training` — whether a key is in use, and what has arrived.
/// Never the key: that was shown once, when it was made.
pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Overview>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    Ok(Json(
        store(&state)?.overview(&person).await.map_err(storage)?,
    ))
}

#[derive(Debug, Serialize)]
pub struct NewKey {
    /// The only time it is ever sent.
    pub key: String,
}

/// `POST /me/training/key` — a new key; any old one stops working.
pub async fn make_key(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<NewKey>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let key = crate::api_token::generate();
    store(&state)?
        .set_key(&person, &hash_key(&key))
        .await
        .map_err(storage)?;
    tracing::info!("[training] {person} made a new ingest key");
    Ok(Json(NewKey { key }))
}

/// `DELETE /me/training/key` — the phone stops getting in. What it sent
/// is kept.
pub async fn revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    store(&state)?.revoke_key(&person).await.map_err(storage)?;
    tracing::info!("[training] {person} revoked their ingest key");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /training/ingest` — Health Auto Export's REST API automation.
///
/// The key is checked before the body is read, so a stranger cannot
/// make Niles take in ten megabytes.
pub async fn ingest(State(state): State<AppState>, request: Request) -> Response {
    let Ok(store) = store(&state) else {
        return refused(StatusCode::NOT_IMPLEMENTED, "training needs a database");
    };
    let Some(key) = crate::auth::presented_token(request.headers()) else {
        return refused(
            StatusCode::UNAUTHORIZED,
            "send your training key as Authorization: Bearer <key>",
        );
    };
    let person = match store.person_for_key(&hash_key(key)).await {
        Ok(Some(person)) => person,
        Ok(None) => return refused(StatusCode::UNAUTHORIZED, "that training key is not in use"),
        Err(e) => return refused(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    if !state.training_limits.allow(&person, Instant::now()) {
        let mut response = refused(StatusCode::TOO_MANY_REQUESTS, "too many posts; slow down");
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, WINDOW.as_secs().into());
        return response;
    }

    let bytes = match axum::body::to_bytes(request.into_body(), MAX_INGEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return refused(
                StatusCode::PAYLOAD_TOO_LARGE,
                "over 10 MiB; switch on Batch Requests in Health Auto Export",
            );
        }
    };
    let body: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(e) => {
            return refused(StatusCode::BAD_REQUEST, &format!("not JSON: {e}"));
        }
    };
    let tz = state
        .config
        .as_ref()
        .and_then(|c| c.current().home.timezone.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::UTC);
    let batch = match niles_training::hae::parse(&body, &tz) {
        Ok(batch) => batch,
        Err(e) => return refused(StatusCode::BAD_REQUEST, &e),
    };

    let saved = async {
        let activities = store.save_activities(&person, &batch.activities).await?;
        let days = store.save_wellness(&person, &batch.wellness).await?;
        store.received(&person, chrono::Utc::now()).await?;
        Ok::<_, niles_training::Error>((activities, days))
    }
    .await;
    let (activities, days) = match saved {
        Ok(saved) => saved,
        Err(e) => return refused(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    tracing::info!(
        "[training] {person}: {} workouts ({} new, {} changed), {} days, {} ignored metrics",
        batch.activities.len(),
        activities.added,
        activities.updated,
        batch.wellness.len(),
        batch.ignored_metrics.len()
    );
    Json(json!({
        "activities": activities,
        "wellness_days": batch.wellness.len(),
        "wellness": days,
        "ignored_metrics": batch.ignored_metrics,
        "unreadable": batch.unreadable,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_training::MemoryTraining;
    use serde_json::Value;
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_TRAINING_SESSION_SECRET";
    const SECRET: &str = "the-training-page-secret";

    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    fn app(store: Arc<MemoryTraining>) -> axum::Router {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_TRAINING_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_TRAINING_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_TRAINING_CLIENT_ID"
github_client_secret_env = "NILES_TEST_TRAINING_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
  {{ email = "guest@example.com" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_training(Some(store as Arc<dyn TrainingStore>));
        crate::server::router(state)
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

    fn by(email: &str, method: &str, uri: &str) -> Request<Body> {
        let token = session::sign(SECRET, &Session::new(email, None));
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, format!("{}={token}", session::COOKIE))
            .body(Body::empty())
            .unwrap()
    }

    fn with_key(key: &str, method: &str, uri: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.into())
            .unwrap()
    }

    const PAYLOAD: &str = include_str!("../../niles-training/fixtures/hae-v2-metric.json");

    async fn key_for(app: &axum::Router, email: &str) -> String {
        let (status, made) = send(app, by(email, "POST", "/me/training/key")).await;
        assert_eq!(status, StatusCode::OK, "{made}");
        made["key"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_key_is_shown_once_and_kept_only_as_a_hash() {
        let store = Arc::new(MemoryTraining::new());
        let app = app(store.clone());
        let key = key_for(&app, "majse@example.com").await;
        assert_eq!(key.len(), 64);

        let (status, overview) = send(&app, by("majse@example.com", "GET", "/me/training")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(overview["key_made_at"].is_string());
        assert!(!overview.to_string().contains(&key), "never read back");

        assert_eq!(
            store
                .person_for_key(&hash_key(&key))
                .await
                .unwrap()
                .as_deref(),
            Some("person:majse")
        );
        assert_eq!(store.person_for_key(&key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_post_lands_with_the_owner_of_the_key_and_says_what_it_did() {
        let store = Arc::new(MemoryTraining::new());
        let app = app(store.clone());
        let key = key_for(&app, "majse@example.com").await;

        let (status, done) = send(&app, with_key(&key, "POST", "/training/ingest", PAYLOAD)).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["activities"]["added"], 1);
        assert_eq!(done["wellness_days"], 2);
        assert_eq!(done["ignored_metrics"], json!(["heart_rate", "step_count"]));

        let (_, again) = send(&app, with_key(&key, "POST", "/training/ingest", PAYLOAD)).await;
        assert_eq!(again["activities"], json!({"added": 0, "updated": 0}));
        assert_eq!(again["wellness"], json!({"added": 0, "updated": 0}));

        let (_, hers) = send(&app, by("majse@example.com", "GET", "/me/training")).await;
        assert_eq!(hers["runs"], 1);
        assert_eq!(hers["wellness_days"], 2);
        assert!(hers["last_received_at"].is_string());
        let (_, his) = send(&app, by("mark@example.com", "GET", "/me/training")).await;
        assert_eq!(his["runs"], 0, "nothing of hers is his");
        assert_eq!(his["key_made_at"], Value::Null);
    }

    #[tokio::test]
    async fn somebody_without_a_voice_is_kept_by_their_address() {
        let store = Arc::new(MemoryTraining::new());
        let app = app(store.clone());
        let key = key_for(&app, "guest@example.com").await;
        assert_eq!(
            store
                .person_for_key(&hash_key(&key))
                .await
                .unwrap()
                .as_deref(),
            Some("email:guest@example.com")
        );
    }

    #[tokio::test]
    async fn a_replaced_or_revoked_key_stops_working() {
        let app = app(Arc::new(MemoryTraining::new()));
        let old = key_for(&app, "majse@example.com").await;
        let new = key_for(&app, "majse@example.com").await;
        let (status, _) = send(&app, with_key(&old, "POST", "/training/ingest", PAYLOAD)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send(&app, with_key(&new, "POST", "/training/ingest", PAYLOAD)).await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = send(&app, by("majse@example.com", "DELETE", "/me/training/key")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, with_key(&new, "POST", "/training/ingest", PAYLOAD)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_training_key_opens_nothing_else() {
        let app = app(Arc::new(MemoryTraining::new()));
        let key = key_for(&app, "majse@example.com").await;
        for (method, uri) in [
            ("GET", "/devices"),
            ("GET", "/me"),
            ("GET", "/me/training"),
            ("POST", "/me/training/key"),
            ("DELETE", "/me/training/key"),
            ("GET", "/auth/api-token"),
            ("GET", "/chat/conversations"),
            ("GET", "/config"),
        ] {
            let (status, _) = send(&app, with_key(&key, method, uri, "")).await;
            assert!(
                status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
                "{method} {uri}: {status}"
            );
        }
    }

    #[tokio::test]
    async fn ingest_without_the_right_key_is_refused() {
        let app = app(Arc::new(MemoryTraining::new()));
        let bare = Request::builder()
            .method("POST")
            .uri("/training/ingest")
            .body(Body::from(PAYLOAD))
            .unwrap();
        assert_eq!(send(&app, bare).await.0, StatusCode::UNAUTHORIZED);
        let wrong = with_key(&"0".repeat(64), "POST", "/training/ingest", PAYLOAD);
        assert_eq!(send(&app, wrong).await.0, StatusCode::UNAUTHORIZED);
        // A session is not a key: the phone is what posts here.
        let mut session = by("majse@example.com", "POST", "/training/ingest");
        *session.body_mut() = Body::from(PAYLOAD);
        assert_eq!(send(&app, session).await.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_refused() {
        let app = app(Arc::new(MemoryTraining::new()));
        let key = key_for(&app, "majse@example.com").await;
        let huge = format!(
            "{{\"data\": {{\"pad\": \"{}\"}}}}",
            "x".repeat(MAX_INGEST_BYTES)
        );
        let (status, _) = send(&app, with_key(&key, "POST", "/training/ingest", huge)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

        // Over axum's own default of two megabytes, and under ours.
        let big = format!(
            "{{\"data\": {{\"pad\": \"{}\"}}}}",
            "x".repeat(3 * 1024 * 1024)
        );
        let (status, _) = send(&app, with_key(&key, "POST", "/training/ingest", big)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn what_is_not_the_export_is_a_bad_request() {
        let app = app(Arc::new(MemoryTraining::new()));
        let key = key_for(&app, "majse@example.com").await;
        let (status, _) = send(&app, with_key(&key, "POST", "/training/ingest", "not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = send(
            &app,
            with_key(&key, "POST", "/training/ingest", "{\"rows\": []}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_phone_in_a_loop_is_slowed_down() {
        let limits = Limits::default();
        let start = Instant::now();
        for _ in 0..POSTS_PER_WINDOW {
            assert!(limits.allow("person:majse", start));
        }
        assert!(!limits.allow("person:majse", start));
        assert!(limits.allow("person:mark", start), "per person");
        assert!(limits.allow("person:majse", start + WINDOW + Duration::from_secs(1)));
    }
}

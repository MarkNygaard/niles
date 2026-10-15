//! A person's own Strava, from their profile. See ARCHITECTURE.md §
//! Training (per person).
//!
//! Each person brings their own Strava API app — a new app may only be
//! used by the athlete who made it — and its Client ID and Secret are
//! theirs alone: saved here, sealed in the database, and never sent
//! back. Connecting is Strava's OAuth sign-in, made safe on the way
//! back the way the calendars' is (see `auth::flow`): a single-use
//! `state`, a cookie binding it to the browser that started (scoped to
//! `/me/strava`), and the person who comes back must be the one who
//! left.
//!
//! Every route here is behind the sign-in gate and acts for the person
//! signed in, never for a person named in the request.

use crate::auth::github::{cookie, header_str, is_secure};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use chrono::{DateTime, Utc};
use niles_training::{Error, Strava, StravaApp, person_key};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const BINDING_COOKIE: &str = "niles_strava";
/// As long as the other sign-ins': enough to sign in to Strava and
/// read its consent screen.
const TTL_SECS: u64 = 10 * 60;
/// Where the browser lands afterwards: the profile. The outcome goes in
/// the query before the hash — the app's routes are the hash.
const PROFILE: &str = "#/me";
const CALLBACK: &str = "/me/strava/callback";

type Failure = (StatusCode, Json<serde_json::Value>);

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, Json(json!({ "error": message.into() })))
}

fn strava(state: &AppState) -> Result<&Arc<Strava>, Failure> {
    state.strava.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "Strava needs Niles's database and its secrets key, and this instance has none",
        )
    })
}

fn refused(e: Error) -> Failure {
    let status = match &e {
        Error::NotConnected | Error::Busy => StatusCode::CONFLICT,
        Error::Invalid { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        Error::RateLimited { until } => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": e.to_string(), "until": until })),
            );
        }
        _ => StatusCode::BAD_GATEWAY,
    };
    failed(status, e.to_string())
}

/// The person signed in to this browser, keyed as their chat is.
///
/// A session only: the house's API key is nobody in particular.
fn signed_in_person(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let config = state
        .config
        .as_ref()
        .ok_or_else(|| {
            failed(
                StatusCode::NOT_IMPLEMENTED,
                "this Niles instance has no people configured",
            )
        })?
        .current();
    let email = crate::auth::signed_in_as(headers, &config).ok_or_else(|| {
        failed(
            StatusCode::UNAUTHORIZED,
            "sign in to connect your own Strava",
        )
    })?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(person_key(speaker.as_deref(), &email))
}

/// What the card shows. Whether a secret is kept, never the secret;
/// no token at all.
#[derive(Debug, Serialize)]
pub struct StravaStatus {
    pub client_id: Option<String>,
    pub secret_set: bool,
    pub connected: bool,
    pub athlete: Option<String>,
    pub last_sync: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub syncing: bool,
    pub paused_until: Option<DateTime<Utc>>,
    pub activities: u64,
    pub runs: u64,
}

async fn status_of(strava: &Strava, person: &str) -> Result<StravaStatus, Failure> {
    let link = strava.store().strava(person).await.map_err(refused)?;
    let counts = strava.store().overview(person).await.map_err(refused)?;
    Ok(StravaStatus {
        secret_set: link
            .as_ref()
            .is_some_and(|l| !l.app.client_secret.is_empty()),
        connected: link.as_ref().is_some_and(|l| l.tokens.is_some()),
        athlete: link
            .as_ref()
            .and_then(|l| l.athlete.as_ref())
            .map(|a| a.name.clone()),
        last_sync: link.as_ref().and_then(|l| l.last_sync),
        last_error: link.as_ref().and_then(|l| l.last_error.clone()),
        client_id: link.map(|l| l.app.client_id),
        syncing: strava.is_syncing(person),
        paused_until: strava.paused_until(person),
        activities: counts.activities,
        runs: counts.runs,
    })
}

/// `GET /me/strava` — the signed-in person's own Strava.
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StravaStatus>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    Ok(Json(status_of(strava(&state)?, &person).await?))
}

#[derive(Debug, Deserialize)]
pub struct AppBody {
    pub client_id: String,
    /// Left out or blank keeps the one saved, for the same app.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// `PUT /me/strava/app` — the Client ID and Secret of the person's own
/// Strava API app.
pub async fn set_app(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AppBody>,
) -> Result<Json<StravaStatus>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let strava = strava(&state)?;
    let client_id = body.client_id.trim();
    if client_id.is_empty()
        || client_id.len() > 20
        || !client_id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the Client ID is the number on strava.com/settings/api",
        ));
    }
    let kept = strava.store().strava(&person).await.map_err(refused)?;
    let same_app = kept.as_ref().filter(|l| l.app.client_id == client_id);
    if kept.as_ref().is_some_and(|l| l.tokens.is_some()) && same_app.is_none() {
        // The tokens belong to the old app; disconnecting revokes them
        // with its secret, which a new one would not.
        return Err(failed(
            StatusCode::CONFLICT,
            "disconnect Strava before changing the app",
        ));
    }
    let given = body
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let client_secret = match (given, same_app) {
        (Some(secret), _) => {
            if secret.len() > 200 || secret.chars().any(char::is_whitespace) {
                return Err(failed(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "that does not look like a Client Secret",
                ));
            }
            secret.to_string()
        }
        (None, Some(link)) => link.app.client_secret.clone(),
        (None, None) => {
            return Err(failed(
                StatusCode::UNPROCESSABLE_ENTITY,
                "the Client Secret is needed too",
            ));
        }
    };
    strava
        .store()
        .set_strava_app(
            &person,
            &StravaApp {
                client_id: client_id.to_string(),
                client_secret,
            },
        )
        .await
        .map_err(refused)?;
    tracing::info!("[strava] {person} saved their Strava app");
    Ok(Json(status_of(strava, &person).await?))
}

/// `DELETE /me/strava` — the app and its connection, revoked at Strava
/// first. The workouts already fetched stay.
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let strava = strava(&state)?;
    match strava.disconnect(&person).await {
        Ok(_) | Err(Error::NotConnected) => {}
        Err(e) => return Err(refused(e)),
    }
    if strava
        .store()
        .remove_strava(&person)
        .await
        .map_err(refused)?
    {
        tracing::info!("[strava] {person} removed their Strava app");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(failed(StatusCode::NOT_FOUND, "no Strava app is saved"))
    }
}

/// `DELETE /me/strava/connection` — Niles stops reading: revoked at
/// Strava, then the tokens are forgotten. The app stays, to connect
/// again with.
pub async fn disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let told = strava(&state)?
        .disconnect(&person)
        .await
        .map_err(|e| match e {
            Error::NotConnected => failed(StatusCode::NOT_FOUND, "Strava is not connected"),
            e => refused(e),
        })?;
    tracing::info!("[strava] {person} disconnected Strava");
    Ok(Json(json!({ "revoked_at_strava": told })))
}

/// `POST /me/strava/sync` — "Sync now": the same sync the poll runs,
/// started in the background.
pub async fn sync_now(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<serde_json::Value>), Failure> {
    let person = signed_in_person(&state, &headers)?;
    let strava = strava(&state)?;
    let connected = strava
        .store()
        .strava(&person)
        .await
        .map_err(refused)?
        .is_some_and(|l| l.tokens.is_some());
    if !connected {
        return Err(refused(Error::NotConnected));
    }
    strava.start(&person).map_err(refused)?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "started": true }))))
}

/// `GET /me/strava/start` — a link the browser follows, not a call the
/// page makes: it ends at Strava.
pub async fn start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let secure = is_secure(&headers);
    let outcome = async {
        let person = signed_in_person(&state, &headers).map_err(reason_of)?;
        let strava = strava(&state).map_err(reason_of)?;
        let link = strava
            .store()
            .strava(&person)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("save your Strava app's Client ID and Client Secret first")?;
        let (flow_state, binding) = state.strava_attempts.begin(person);
        let to = strava.client().authorize_url(
            &link.app.client_id,
            &redirect_uri(&headers),
            &flow_state,
        );
        Ok::<_, String>((to, binding))
    }
    .await;
    match outcome {
        Ok((to, binding)) => (
            StatusCode::SEE_OTHER,
            AppendHeaders([
                (header::LOCATION, to),
                (header::SET_COOKIE, binding_cookie(&binding, secure)),
            ]),
        )
            .into_response(),
        Err(reason) => back(
            &format!("strava_error={}", urlencoding::encode(&reason)),
            secure,
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    /// What the person left ticked on Strava's consent screen.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /me/strava/callback` — where Strava sends the browser back.
/// Always ends on the profile, saying how it went.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let secure = is_secure(&headers);
    match connect(&state, &headers, query).await {
        Ok(name) => back(
            &format!("strava=connected&name={}", urlencoding::encode(&name)),
            secure,
        ),
        Err(reason) => {
            tracing::warn!("[strava] connecting failed: {reason}");
            back(
                &format!("strava_error={}", urlencoding::encode(&reason)),
                secure,
            )
        }
    }
}

async fn connect(
    state: &AppState,
    headers: &HeaderMap,
    query: CallbackQuery,
) -> Result<String, String> {
    if let Some(error) = query.error {
        return Err(if error == "access_denied" {
            "Strava was not given access".to_string()
        } else {
            error
        });
    }
    let (Some(code), Some(flow_state)) = (query.code, query.state) else {
        return Err("nothing came back from Strava".into());
    };
    let started_by = state
        .strava_attempts
        .finish(&flow_state, cookie(headers, BINDING_COOKIE).as_deref())
        .ok_or("that sign-in expired or was started elsewhere; try again")?
        .next;
    let person = signed_in_person(state, headers).map_err(reason_of)?;
    if person != started_by {
        return Err("signed in as somebody else than when it started".into());
    }
    if !niles_training::strava::scope_allows_activities(query.scope.as_deref().unwrap_or("")) {
        return Err(
            "Strava was not allowed to share your activities; connect again and leave \
             them ticked"
                .into(),
        );
    }
    let strava = strava(state).map_err(reason_of)?;
    let athlete = strava
        .connect(&person, &code)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("[strava] {person} connected Strava as {}", athlete.name);
    // The backfill, in the background: half a year can take longer
    // than a browser should wait, and longer than one 15-minute budget.
    if let Err(e) = strava.start(&person) {
        tracing::warn!("[strava] {person}: the first sync did not start: {e}");
    }
    Ok(athlete.name)
}

/// Strava checks only that it is on the app's Authorization Callback
/// Domain, which is why the card names this host.
fn redirect_uri(headers: &HeaderMap) -> String {
    let host = header_str(headers, "x-forwarded-host")
        .or_else(|| header_str(headers, header::HOST))
        .and_then(|h| h.split(',').next())
        .unwrap_or("localhost")
        .trim();
    let scheme = if is_secure(headers) { "https" } else { "http" };
    format!("{scheme}://{host}{CALLBACK}")
}

fn binding_cookie(value: &str, secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}={value}; Path=/me/strava; HttpOnly; SameSite=Lax; Max-Age={TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn clear_binding_cookie(secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}=; Path=/me/strava; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// To the profile, with the outcome for it to say.
fn back(outcome: &str, secure: bool) -> Response {
    (
        StatusCode::SEE_OTHER,
        AppendHeaders([
            (header::LOCATION, format!("/?{outcome}{PROFILE}")),
            (header::SET_COOKIE, clear_binding_cookie(secure)),
        ]),
    )
        .into_response()
}

fn reason_of((_, body): Failure) -> String {
    body.0
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("something went wrong")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_training::{MemoryTraining, TrainingStore};
    use serde_json::Value;
    use tower::ServiceExt;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SECRET_VAR: &str = "NILES_TEST_STRAVA_SESSION_SECRET";
    const SECRET: &str = "the-strava-page-secret";
    const HER_SECRET: &str = "0123456789abcdef0123456789abcdef01234567";
    const MAJSE: &str = "person:majse";

    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    struct Fixture {
        app: axum::Router,
        store: Arc<MemoryTraining>,
        strava: MockServer,
    }

    async fn fixture() -> Fixture {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_STRAVA_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_STRAVA_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_STRAVA_CLIENT_ID"
github_client_secret_env = "NILES_TEST_STRAVA_CLIENT_SECRET"
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
        let server = MockServer::start().await;
        // The backfill a connection starts; what it finds is not what
        // these tests are about.
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        let store = Arc::new(MemoryTraining::new());
        let strava = Arc::new(Strava::new(
            store.clone() as Arc<dyn TrainingStore>,
            niles_training::strava::Client::with_base(reqwest::Client::new(), &server.uri()),
        ));
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_strava(Some(strava));
        Fixture {
            app: crate::server::router(state),
            store,
            strava: server,
        }
    }

    fn session_cookie(email: &str) -> String {
        format!(
            "{}={}",
            session::COOKIE,
            session::sign(SECRET, &Session::new(email, None))
        )
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, session_cookie(email))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::HOST, "niles.example")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn save_app(f: &Fixture, email: &str) {
        let (status, _, body) = send(
            &f.app,
            by(
                email,
                "PUT",
                "/me/strava/app",
                Some(json!({ "client_id": "1001", "client_secret": HER_SECRET })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// Starts a sign-in as `email`: Strava's address, and the binding
    /// cookie.
    async fn begin(f: &Fixture, email: &str) -> (String, String) {
        let (status, headers, _) = send(&f.app, by(email, "GET", "/me/strava/start", None)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let to = headers[header::LOCATION].to_str().unwrap().to_string();
        let binding = headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let state = to
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();
        (state, binding)
    }

    fn come_back(email: &str, state: &str, binding: &str, scope: &str) -> Request<Body> {
        Request::builder()
            .uri(format!(
                "/me/strava/callback?state={state}&code=the-code&scope={scope}"
            ))
            .header(
                header::COOKIE,
                format!("{}; {binding}", session_cookie(email)),
            )
            .body(Body::empty())
            .unwrap()
    }

    async fn strava_accepts_the_code(f: &Fixture, times: u64) {
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains(format!("client_secret={HER_SECRET}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "token_type": "Bearer",
                "access_token": "her-access-token",
                "refresh_token": "her-refresh-token",
                "expires_at": (Utc::now() + chrono::Duration::hours(6)).timestamp(),
                "athlete": { "id": 42, "firstname": "Majse", "lastname": "H" }
            })))
            .expect(times)
            .mount(&f.strava)
            .await;
    }

    fn location(headers: &HeaderMap) -> String {
        headers[header::LOCATION].to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn connecting_goes_to_strava_and_comes_back_connected() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (status, headers, _) = send(
            &f.app,
            by("majse@example.com", "GET", "/me/strava/start", None),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let to = location(&headers);
        assert!(to.starts_with(&format!(
            "{}/oauth/authorize?client_id=1001&",
            f.strava.uri()
        )));
        assert!(to.contains("scope=activity%3Aread_all"));
        assert!(to.contains("redirect_uri=http%3A%2F%2Fniles.example%2Fme%2Fstrava%2Fcallback"));
        let cookie = headers[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("Path=/me/strava;"), "{cookie}");

        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 1).await;
        let (status, headers, _) = send(
            &f.app,
            come_back(
                "majse@example.com",
                &state,
                &binding,
                "read,activity:read_all",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(location(&headers), "/?strava=connected&name=Majse%20H#/me");

        let (_, _, status) = send(&f.app, by("majse@example.com", "GET", "/me/strava", None)).await;
        assert_eq!(status["connected"], true);
        assert_eq!(status["athlete"], "Majse H");
        assert_eq!(status["client_id"], "1001");
        assert_eq!(status["secret_set"], true);
        for secret in [HER_SECRET, "her-access-token", "her-refresh-token"] {
            assert!(!status.to_string().contains(secret), "never sent back");
        }
    }

    #[tokio::test]
    async fn somebody_else_cannot_finish_her_sign_in() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        save_app(&f, "mark@example.com").await;
        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 0).await;

        // Her state and her browser's binding, but his session.
        let (_, headers, _) = send(
            &f.app,
            come_back("mark@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        assert!(location(&headers).contains("strava_error="), "{headers:?}");
        for person in [MAJSE, "person:mark"] {
            let link = f.store.strava(person).await.unwrap().unwrap();
            assert!(link.tokens.is_none(), "{person} got tokens");
        }
        // And the state is spent: she cannot use it after him either.
        let (_, headers, _) = send(
            &f.app,
            come_back("majse@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        assert!(location(&headers).contains("expired"), "{headers:?}");
    }

    #[tokio::test]
    async fn a_callback_without_the_browser_that_started_it_is_refused() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (state, _) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 0).await;
        let (_, headers, _) = send(
            &f.app,
            come_back(
                "majse@example.com",
                &state,
                "niles_strava=somebody-elses",
                "activity:read_all",
            ),
        )
        .await;
        assert!(location(&headers).contains("strava_error="));
        assert!(f.store.strava_connected().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn without_activities_ticked_it_does_not_connect() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 0).await;
        let (_, headers, _) = send(
            &f.app,
            come_back("majse@example.com", &state, &binding, "read"),
        )
        .await;
        assert!(location(&headers).contains("not%20allowed%20to%20share"));
    }

    #[tokio::test]
    async fn somebody_without_a_voice_connects_by_their_address() {
        let f = fixture().await;
        save_app(&f, "guest@example.com").await;
        let (state, binding) = begin(&f, "guest@example.com").await;
        strava_accepts_the_code(&f, 1).await;
        send(
            &f.app,
            come_back("guest@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        assert_eq!(
            f.store.strava_connected().await.unwrap(),
            ["email:guest@example.com"]
        );
    }

    #[tokio::test]
    async fn the_app_s_secret_is_kept_but_never_shown() {
        let f = fixture().await;
        let (status, _, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                "/me/strava/app",
                Some(json!({ "client_id": "1001" })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "a secret is needed"
        );
        let (status, _, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                "/me/strava/app",
                Some(json!({ "client_id": "not a number", "client_secret": "x" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        save_app(&f, "majse@example.com").await;
        // The same app again with the secret left blank keeps it.
        let (status, _, shown) = send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                "/me/strava/app",
                Some(json!({ "client_id": "1001", "client_secret": "" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(shown["secret_set"], true);
        assert!(!shown.to_string().contains(HER_SECRET));
        let kept = f.store.strava(MAJSE).await.unwrap().unwrap();
        assert_eq!(kept.app.client_secret, HER_SECRET);

        // Nobody else sees that she has one.
        let (_, _, his) = send(&f.app, by("mark@example.com", "GET", "/me/strava", None)).await;
        assert_eq!(his["client_id"], Value::Null);
        assert_eq!(his["secret_set"], false);
    }

    #[tokio::test]
    async fn the_app_cannot_change_under_a_connection() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 1).await;
        send(
            &f.app,
            come_back("majse@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        let (status, _, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                "/me/strava/app",
                Some(json!({ "client_id": "2002", "client_secret": "another" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn sync_now_needs_a_connection_and_then_starts() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (status, _, _) = send(
            &f.app,
            by("majse@example.com", "POST", "/me/strava/sync", None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 1).await;
        send(
            &f.app,
            come_back("majse@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        // The backfill the connection started may still hold the claim.
        for _ in 0..50 {
            let (status, _, _) = send(
                &f.app,
                by("majse@example.com", "POST", "/me/strava/sync", None),
            )
            .await;
            if status == StatusCode::ACCEPTED {
                return;
            }
            assert_eq!(status, StatusCode::CONFLICT);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("sync now never started");
    }

    #[tokio::test]
    async fn disconnecting_revokes_and_keeps_the_app() {
        let f = fixture().await;
        save_app(&f, "majse@example.com").await;
        let (state, binding) = begin(&f, "majse@example.com").await;
        strava_accepts_the_code(&f, 1).await;
        send(
            &f.app,
            come_back("majse@example.com", &state, &binding, "activity:read_all"),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .and(body_string_contains("token=her-refresh-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&f.strava)
            .await;

        let (status, _, body) = send(
            &f.app,
            by("majse@example.com", "DELETE", "/me/strava/connection", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["revoked_at_strava"], true);
        let (_, _, shown) = send(&f.app, by("majse@example.com", "GET", "/me/strava", None)).await;
        assert_eq!(shown["connected"], false);
        assert_eq!(shown["secret_set"], true);
        assert_eq!(shown["athlete"], Value::Null);

        let (status, _, _) = send(
            &f.app,
            by("majse@example.com", "DELETE", "/me/strava", None),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(f.store.strava(MAJSE).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_iphone_routes_are_gone() {
        let f = fixture().await;
        for (method, uri) in [
            ("POST", "/training/ingest"),
            ("POST", "/me/training/key"),
            ("DELETE", "/me/training/key"),
            ("GET", "/me/training"),
        ] {
            let (status, _, _) = send(&f.app, by("majse@example.com", method, uri, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn nothing_here_answers_without_a_session() {
        let f = fixture().await;
        for (method, uri) in [
            ("GET", "/me/strava"),
            ("PUT", "/me/strava/app"),
            ("GET", "/me/strava/start"),
            ("GET", "/me/strava/callback"),
            ("POST", "/me/strava/sync"),
            ("DELETE", "/me/strava/connection"),
            ("POST", "/training/ingest"),
        ] {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            let (status, _, _) = send(&f.app, req).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }

    #[test]
    fn the_binding_cookie_is_only_sent_back_here() {
        let set = binding_cookie("abc", true);
        assert!(set.contains("Path=/me/strava;"));
        assert!(set.contains("HttpOnly"));
        assert!(set.ends_with("; Secure"));
    }
}

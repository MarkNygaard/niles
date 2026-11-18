//! My profile → Schedules: each person's own agent tasks, which the
//! assistant answers on a schedule, as that person, in their chat.
//!
//! Every route answers the signed-in person only; another person's
//! schedule answers like one never kept.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use niles_schedules::{NewSchedule, Run, Schedule, When};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

/// What runs the schedules. Implemented by the binary, which knows the
/// household's time zone and its assistant.
#[async_trait]
pub trait PersonalSchedules: Send + Sync {
    /// Whether an assistant is switched on: without one, nothing runs.
    fn assistant(&self) -> bool;
    async fn list(&self, person: &str) -> Result<Vec<Schedule>, niles_schedules::Error>;
    async fn add(&self, person: &str, new: NewSchedule)
    -> Result<Schedule, niles_schedules::Error>;
    async fn update(
        &self,
        person: &str,
        id: i64,
        new: NewSchedule,
    ) -> Result<Option<Schedule>, niles_schedules::Error>;
    async fn remove(&self, person: &str, id: i64) -> Result<bool, niles_schedules::Error>;
    async fn pause(
        &self,
        person: &str,
        id: i64,
        paused: bool,
    ) -> Result<bool, niles_schedules::Error>;
    /// Runs it now, in the background; false when the id isn't theirs.
    async fn run_now(&self, person: &str, id: i64) -> Result<bool, niles_schedules::Error>;
    async fn runs(&self, person: &str, id: i64)
    -> Result<Option<Vec<Run>>, niles_schedules::Error>;
}

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, message.into())
}

fn schedules(state: &AppState) -> Result<&dyn PersonalSchedules, Failure> {
    state.schedules.as_deref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "Schedules need Niles's database, and this Niles doesn't have one.",
        )
    })
}

/// The person signed in to this browser, keyed as their chat is.
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
            "sign in to keep your own schedules",
        )
    })?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(niles_training::person_key(speaker.as_deref(), &email))
}

fn refused(e: niles_schedules::Error) -> Failure {
    match e {
        niles_schedules::Error::Invalid { kind, reason } => failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("the {kind}: {reason}"),
        ),
        why @ niles_schedules::Error::Storage(_) => {
            tracing::warn!("[schedules] {why}");
            failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "your schedules couldn't be read or written",
            )
        }
    }
}

const NO_SUCH: &str = "no such schedule";

#[derive(Debug, Deserialize)]
pub struct ScheduleBody {
    pub name: String,
    pub instruction: String,
    pub when: When,
    /// The saved login its runs may act on without asking (NIL-87).
    #[serde(default)]
    pub acts_on: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PauseBody {
    pub paused: bool,
}

fn read<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Failure> {
    body.map(|Json(b)| b).map_err(|_| {
        failed(
            StatusCode::BAD_REQUEST,
            "send the schedule as JSON with its name, instruction and when it runs",
        )
    })
}

#[derive(Serialize)]
pub struct ScheduleList {
    pub schedules: Vec<Schedule>,
    /// Whether an assistant is switched on to run them.
    pub assistant: bool,
}

/// `GET /me/schedules`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ScheduleList>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let schedules = schedules(&state)?;
    Ok(Json(ScheduleList {
        schedules: schedules.list(&person).await.map_err(refused)?,
        assistant: schedules.assistant(),
    }))
}

/// `POST /me/schedules`
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ScheduleBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Schedule>), Failure> {
    let person = signed_in_person(&state, &headers)?;
    let schedules = schedules(&state)?;
    let b = read(body)?;
    let new = NewSchedule::new(&b.name, &b.instruction, b.when)
        .map_err(refused)?
        .acting_on(b.acts_on);
    let kept = schedules.add(&person, new).await.map_err(refused)?;
    tracing::info!("[schedules] {person} added schedule {}", kept.id);
    Ok((StatusCode::CREATED, Json(kept)))
}

/// `PUT /me/schedules/{id}`
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Result<Json<ScheduleBody>, JsonRejection>,
) -> Result<Json<Schedule>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let schedules = schedules(&state)?;
    let b = read(body)?;
    let new = NewSchedule::new(&b.name, &b.instruction, b.when)
        .map_err(refused)?
        .acting_on(b.acts_on);
    schedules
        .update(&person, id, new)
        .await
        .map_err(refused)?
        .map(Json)
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, NO_SUCH))
}

/// `DELETE /me/schedules/{id}`
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    if !schedules(&state)?
        .remove(&person, id)
        .await
        .map_err(refused)?
    {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/schedules/{id}/pause`
pub async fn pause(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Result<Json<PauseBody>, JsonRejection>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let schedules = schedules(&state)?;
    let b = read(body)?;
    if !schedules
        .pause(&person, id, b.paused)
        .await
        .map_err(refused)?
    {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/schedules/{id}/run` — now, once, in the background.
pub async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let schedules = schedules(&state)?;
    if !schedules.assistant() {
        return Err(failed(
            StatusCode::CONFLICT,
            "Schedules run on Claude or ChatGPT; switch one on under Settings first.",
        ));
    }
    if !schedules.run_now(&person, id).await.map_err(refused)? {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Serialize)]
pub struct RunList {
    pub runs: Vec<Run>,
}

/// `GET /me/schedules/{id}/runs`
pub async fn runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<RunList>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let runs = schedules(&state)?
        .runs(&person, id)
        .await
        .map_err(refused)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, NO_SUCH))?;
    Ok(Json(RunList { runs }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use niles_schedules::{MemorySchedules, ScheduleStore};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_SCHEDULES_SESSION_SECRET";
    const SECRET: &str = "the-schedules-secret";
    const TOKEN_VAR: &str = "NILES_TEST_SCHEDULES_API_TOKEN";
    const MAJSE: Option<&str> = Some("majse@example.com");
    const MARK: Option<&str> = Some("mark@example.com");

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// The binary's part, over the store, run by nobody.
    struct Fake {
        store: MemorySchedules,
        assistant: bool,
    }

    fn next() -> Option<chrono::DateTime<chrono::Utc>> {
        Some(chrono::Utc::now() + chrono::TimeDelta::hours(1))
    }

    #[async_trait]
    impl PersonalSchedules for Fake {
        fn assistant(&self) -> bool {
            self.assistant
        }
        async fn list(&self, person: &str) -> Result<Vec<Schedule>, niles_schedules::Error> {
            self.store.list(person).await
        }
        async fn add(
            &self,
            person: &str,
            new: NewSchedule,
        ) -> Result<Schedule, niles_schedules::Error> {
            self.store.add(person, &new, next()).await
        }
        async fn update(
            &self,
            person: &str,
            id: i64,
            new: NewSchedule,
        ) -> Result<Option<Schedule>, niles_schedules::Error> {
            self.store.update(person, id, &new, next()).await
        }
        async fn remove(&self, person: &str, id: i64) -> Result<bool, niles_schedules::Error> {
            self.store.remove(person, id).await
        }
        async fn pause(
            &self,
            person: &str,
            id: i64,
            paused: bool,
        ) -> Result<bool, niles_schedules::Error> {
            self.store.set_paused(person, id, paused, next()).await
        }
        async fn run_now(&self, person: &str, id: i64) -> Result<bool, niles_schedules::Error> {
            Ok(self.store.get(person, id).await?.is_some())
        }
        async fn runs(
            &self,
            person: &str,
            id: i64,
        ) -> Result<Option<Vec<Run>>, niles_schedules::Error> {
            self.store.runs(person, id).await
        }
    }

    fn app(assistant: bool) -> axum::Router {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_SCHEDULES_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_SCHEDULES_CLIENT_SECRET", "secret");
            std::env::set_var(TOKEN_VAR, "the-house-token");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_SCHEDULES_CLIENT_ID"
github_client_secret_env = "NILES_TEST_SCHEDULES_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
api_token_env = "{TOKEN_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
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
        .with_schedules(Some(Arc::new(Fake {
            store: MemorySchedules::new(),
            assistant,
        })));
        crate::server::router(state)
    }

    fn by(email: Option<&str>, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(email) = email {
            let session = crate::auth::session::Session::new(email, None);
            let cookie = format!(
                "{}={}",
                crate::auth::session::COOKIE,
                crate::auth::session::sign(SECRET, &session)
            );
            req = req.header(header::COOKIE, cookie);
        }
        req.body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn linear() -> Value {
        json!({
            "name": "Linear on Monday",
            "instruction": "my open Linear issues by priority",
            "when": { "kind": "days", "days": ["Mon"], "at": "08:00" },
        })
    }

    async fn added(app: &axum::Router) -> i64 {
        let (status, added) = send(app, by(MAJSE, "POST", "/me/schedules", Some(linear()))).await;
        assert_eq!(status, StatusCode::CREATED, "{added}");
        serde_json::from_str::<Value>(&added).unwrap()["id"]
            .as_i64()
            .unwrap()
    }

    #[tokio::test]
    async fn a_schedule_is_its_persons_alone() {
        let app = app(true);
        let id = added(&app).await;
        let (_, hers) = send(&app, by(MAJSE, "GET", "/me/schedules", None)).await;
        assert!(hers.contains("Linear on Monday"), "{hers}");
        let (_, his) = send(&app, by(MARK, "GET", "/me/schedules", None)).await;
        assert_eq!(
            serde_json::from_str::<Value>(&his).unwrap()["schedules"],
            json!([])
        );
        for (method, uri, body) in [
            ("PUT", format!("/me/schedules/{id}"), Some(linear())),
            ("DELETE", format!("/me/schedules/{id}"), None),
            (
                "POST",
                format!("/me/schedules/{id}/pause"),
                Some(json!({ "paused": true })),
            ),
            ("POST", format!("/me/schedules/{id}/run"), None),
            ("GET", format!("/me/schedules/{id}/runs"), None),
        ] {
            let (status, said) = send(&app, by(MARK, method, &uri, body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {said}");
        }
        let run = format!("/me/schedules/{id}/run");
        let (status, _) = send(&app, by(MAJSE, "POST", &run, None)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn more_often_than_hourly_is_refused() {
        let app = app(true);
        let too_often = json!({
            "name": "Mail",
            "instruction": "anything new?",
            "when": { "kind": "every_hours", "hours": 0 },
        });
        let (status, said) = send(&app, by(MAJSE, "POST", "/me/schedules", Some(too_often))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{said}");
        let half = json!({ "name": "x" });
        let (status, _) = send(&app, by(MAJSE, "POST", "/me/schedules", Some(half))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn without_an_assistant_nothing_runs_now_and_the_list_says_so() {
        let app = app(false);
        let id = added(&app).await;
        let run = format!("/me/schedules/{id}/run");
        let (status, _) = send(&app, by(MAJSE, "POST", &run, None)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (_, list) = send(&app, by(MAJSE, "GET", "/me/schedules", None)).await;
        assert_eq!(
            serde_json::from_str::<Value>(&list).unwrap()["assistant"],
            false
        );
    }

    #[tokio::test]
    async fn signed_out_there_are_no_schedules() {
        let app = app(true);
        let (status, _) = send(&app, by(None, "GET", "/me/schedules", None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

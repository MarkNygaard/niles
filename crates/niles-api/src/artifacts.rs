//! The signed-in person's artifacts: the Saved tab of Niles's panel.
//! See ARCHITECTURE.md § Artifacts (per person).
//!
//! Every route acts for the caller, by the key their chat is kept
//! under, and never for a person named in the request: another
//! person's id is a 404, the same as one that does not exist.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use niles_artifacts::{
    Artifact, ArtifactStore, Author, Changes, Filter, NewRevision, Revision, RevisionMeta,
};
use niles_core::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// What only the binary can do for an artifact: run its job again, and
/// say which assistant would.
#[async_trait]
pub trait Jobs: Send + Sync {
    /// `claude` or `chatgpt`, whichever would make an artifact now;
    /// `None` when neither is on and able to run.
    fn assistant(&self) -> Option<&'static str>;

    /// Start a failed job again, on the same artifact with the same
    /// brief.
    async fn retry(&self, person: &str, id: i64) -> Result<Artifact, Refused>;
}

/// Why a job could not be started again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    NotFound,
    /// It did not fail, or it was never a job.
    NotRetryable(String),
    /// No assistant is on, or the person has as many jobs going as they
    /// may.
    Unavailable(String),
    Storage(String),
}

type Failure = (StatusCode, Json<Value>);

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, Json(json!({ "error": message.into() })))
}

fn not_found() -> Failure {
    failed(StatusCode::NOT_FOUND, "no such artifact")
}

fn storage(e: niles_artifacts::Error) -> Failure {
    match e {
        niles_artifacts::Error::Invalid { .. } => {
            failed(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
        }
        niles_artifacts::Error::Storage(_) => failed(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

fn store(state: &AppState) -> Result<&Arc<dyn ArtifactStore>, Failure> {
    state.artifacts.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "saved artifacts need Niles's database, and this instance has none",
        )
    })
}

/// Tell the person's open pages, so another tab or phone shows it too.
pub fn announce(bus: &niles_core::EventBus, artifact: &Artifact) {
    bus.publish(Event::ArtifactChanged {
        person: artifact.person.clone(),
        id: artifact.id,
        kind: artifact.kind.clone(),
        title: artifact.title.clone(),
        status: artifact.status.as_str().into(),
        status_text: artifact.status_text.clone(),
    });
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Listed {
    pub artifacts: Vec<Artifact>,
    /// Which assistant makes them now; `None` is why nothing new will be
    /// made, which the Saved tab explains.
    pub assistant: Option<&'static str>,
}

/// `GET /me/artifacts`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Listed>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let artifacts = store
        .list(
            &person,
            Filter {
                pinned: query.pinned,
                archived: query.archived,
            },
        )
        .await
        .map_err(storage)?;
    Ok(Json(Listed {
        artifacts,
        assistant: state.artifact_jobs.as_ref().and_then(|j| j.assistant()),
    }))
}

#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub artifact: Artifact,
    /// The current version's content; `None` before there is one.
    pub content: Option<Value>,
    /// Newest first.
    pub revisions: Vec<RevisionMeta>,
}

async fn detail(
    store: &dyn ArtifactStore,
    person: &str,
    artifact: Artifact,
) -> Result<Detail, Failure> {
    let content = match artifact.current_revision {
        0 => None,
        n => store
            .revision(person, artifact.id, n)
            .await
            .map_err(storage)?
            .map(|r| r.content),
    };
    let revisions = store
        .revisions(person, artifact.id)
        .await
        .map_err(storage)?;
    Ok(Detail {
        artifact,
        content,
        revisions,
    })
}

/// `GET /me/artifacts/{id}`
pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Detail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let artifact = store
        .get(&person, id)
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    Ok(Json(detail(store.as_ref(), &person, artifact).await?))
}

/// `GET /me/artifacts/{id}/revisions/{n}`
pub async fn revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, n)): Path<(i64, i32)>,
) -> Result<Json<Revision>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    store
        .revision(&person, id, n)
        .await
        .map_err(storage)?
        .map(Json)
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such version"))
}

/// Each field: absent leaves it. `content` saves a version by the
/// person, with `reason`.
#[derive(Debug, Default, Deserialize)]
pub struct Patch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub notify: Option<bool>,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `PATCH /me/artifacts/{id}`
pub async fn patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<Patch>,
) -> Result<Json<Detail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let artifact = store
        .get(&person, id)
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    // Everything checked before anything is written, so a bad title does
    // not leave the content saved and the page saying it failed.
    let title = body
        .title
        .as_deref()
        .map(niles_artifacts::title)
        .transpose()
        .map_err(storage)?;
    let content = body
        .content
        .as_ref()
        .map(|c| niles_artifacts::kinds::validate(&artifact.kind, c))
        .transpose()
        .map_err(storage)?;
    let mut artifact = store
        .change(
            &person,
            id,
            Changes {
                title,
                pinned: body.pinned,
                archived: body.archived,
                notify: body.notify,
            },
        )
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    if let Some(content) = content {
        let reason = body
            .reason
            .as_deref()
            .map(niles_artifacts::summary)
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "Edited in the app".into());
        artifact = store
            .revise(
                &person,
                id,
                NewRevision {
                    content,
                    reason,
                    author: Author::Person,
                    summary: None,
                },
            )
            .await
            .map_err(storage)?
            .ok_or_else(not_found)?;
    }
    announce(&state.event_bus, &artifact);
    Ok(Json(detail(store.as_ref(), &person, artifact).await?))
}

/// `POST /me/artifacts/{id}/restore/{n}`
pub async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, n)): Path<(i64, i32)>,
) -> Result<Json<Detail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let artifact = niles_artifacts::restore(store.as_ref(), &person, id, n)
        .await
        .map_err(storage)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such version"))?;
    announce(&state.event_bus, &artifact);
    Ok(Json(detail(store.as_ref(), &person, artifact).await?))
}

/// `POST /me/artifacts/{id}/retry`
pub async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Artifact>, Failure> {
    store(&state)?;
    let jobs = state.artifact_jobs.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance runs no jobs",
        )
    })?;
    let person = crate::chat::person_of(&state, &headers);
    match jobs.retry(&person, id).await {
        Ok(artifact) => Ok(Json(artifact)),
        Err(Refused::NotFound) => Err(not_found()),
        Err(Refused::NotRetryable(why)) => Err(failed(StatusCode::CONFLICT, why)),
        Err(Refused::Unavailable(why)) => Err(failed(StatusCode::SERVICE_UNAVAILABLE, why)),
        Err(Refused::Storage(why)) => Err(failed(StatusCode::BAD_GATEWAY, why)),
    }
}

/// `DELETE /me/artifacts/{id}`
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let Some(artifact) = store.get(&person, id).await.map_err(storage)? else {
        return Err(not_found());
    };
    if !store.delete(&person, id).await.map_err(storage)? {
        return Err(not_found());
    }
    state.event_bus.publish(Event::ArtifactChanged {
        person,
        id,
        kind: artifact.kind,
        title: artifact.title,
        status: "deleted".into(),
        status_text: None,
    });
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use niles_artifacts::{MemoryArtifacts, NewArtifact, Status};
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_ARTIFACTS_SESSION_SECRET";
    const SECRET: &str = "the-artifacts-page-secret";
    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// Says which assistant is on, and records what it was asked to run
    /// again.
    #[derive(Default)]
    struct FakeJobs {
        on: bool,
        retried: std::sync::Mutex<Vec<(String, i64)>>,
    }

    #[async_trait]
    impl Jobs for FakeJobs {
        fn assistant(&self) -> Option<&'static str> {
            self.on.then_some("claude")
        }

        async fn retry(&self, person: &str, id: i64) -> Result<Artifact, Refused> {
            self.retried.lock().unwrap().push((person.into(), id));
            Err(Refused::NotRetryable("it did not fail".into()))
        }
    }

    struct Fixture {
        app: axum::Router,
        store: Arc<MemoryArtifacts>,
        jobs: Arc<FakeJobs>,
        bus: niles_core::EventBus,
    }

    fn fixture(assistant_on: bool) -> Fixture {
        // SAFETY: a test-only variable nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_ARTIFACTS_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_ARTIFACTS_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_ARTIFACTS_CLIENT_ID"
github_client_secret_env = "NILES_TEST_ARTIFACTS_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let store = Arc::new(MemoryArtifacts::new());
        let jobs = Arc::new(FakeJobs {
            on: assistant_on,
            ..FakeJobs::default()
        });
        let bus = niles_core::EventBus::default();
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            bus.clone(),
        )
        .with_config_store(Some(config))
        .with_artifacts(
            Some(store.clone() as Arc<dyn ArtifactStore>),
            Some(jobs.clone() as Arc<dyn Jobs>),
        );
        Fixture {
            app: crate::server::router(state),
            store,
            jobs,
            bus,
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

    async fn make(store: &MemoryArtifacts, person: &str, title: &str, markdown: &str) -> i64 {
        store
            .create(
                person,
                NewArtifact {
                    kind: "document".into(),
                    title: title.into(),
                    summary: "a summary".into(),
                    notify: true,
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: None,
                    conversation_id: None,
                    first: Some(NewRevision {
                        content: json!({ "markdown": markdown }),
                        reason: "First draft".into(),
                        author: Author::Claude,
                        summary: None,
                    }),
                },
            )
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn the_list_is_the_callers_own_with_whether_an_assistant_is_on() {
        let f = fixture(true);
        make(&f.store, MARK, "Packing list", "- [ ] passport").await;
        make(&f.store, MAJSE, "Her plan", "# hers").await;
        let (status, body) =
            send(&f.app, by("mark@example.com", "GET", "/me/artifacts", None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let titles: Vec<&str> = body["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["Packing list"]);
        assert_eq!(body["assistant"], "claude");
        assert_eq!(
            body["artifacts"][0]["preview"]["lines"][0]["style"],
            "check"
        );
        assert!(body["artifacts"][0].get("person").is_none());
    }

    #[tokio::test]
    async fn without_an_assistant_the_list_says_so() {
        let f = fixture(false);
        let (_, body) = send(&f.app, by("mark@example.com", "GET", "/me/artifacts", None)).await;
        assert_eq!(body["assistant"], Value::Null);
    }

    #[tokio::test]
    async fn another_persons_artifact_is_not_found_on_every_route() {
        let f = fixture(true);
        let id = make(&f.store, MAJSE, "Her plan", "# hers").await;
        let mark = "mark@example.com";
        for (method, uri, body) in [
            ("GET", format!("/me/artifacts/{id}"), None),
            ("GET", format!("/me/artifacts/{id}/revisions/1"), None),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "title": "Mine now" })),
            ),
            ("POST", format!("/me/artifacts/{id}/restore/1"), None),
            ("DELETE", format!("/me/artifacts/{id}"), None),
        ] {
            let (status, _) = send(&f.app, by(mark, method, &uri, body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
        let hers = f.store.get(MAJSE, id).await.unwrap().unwrap();
        assert_eq!(hers.title, "Her plan");
        assert_eq!(hers.current_revision, 1);
    }

    #[tokio::test]
    async fn ticking_a_checklist_item_saves_a_version_by_the_person() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Packing list", "- [ ] passport").await;
        let mut events = f.bus.subscribe();
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({
                    "content": { "markdown": "- [x] passport" },
                    "reason": "Ticked passport",
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["content"]["markdown"], "- [x] passport");
        assert_eq!(body["current_revision"], 2);
        assert_eq!(body["revisions"][0]["author"], "person");
        assert_eq!(body["revisions"][0]["reason"], "Ticked passport");
        match events.try_recv().unwrap() {
            Event::ArtifactChanged {
                person, id: got, ..
            } => {
                assert_eq!(person, MARK);
                assert_eq!(got, id);
            }
            other => panic!("expected an artifact event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn content_that_does_not_fit_its_kind_is_refused_and_nothing_changes() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Packing list", "- [ ] passport").await;
        let (status, _) = send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "title": "Renamed", "content": { "markdown": "  " } })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!(a.title, "Packing list");
    }

    #[tokio::test]
    async fn pin_archive_notify_and_rename() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Packing list", "- a").await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({
                    "title": " Cabin packing ",
                    "pinned": true,
                    "archived": true,
                    "notify": false,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["title"], "Cabin packing");
        assert_eq!(body["pinned"], true);
        assert_eq!(body["archived"], true);
        assert_eq!(body["notify"], false);
        assert_eq!(body["current_revision"], 1, "no new version");
        let (_, listed) = send(
            &f.app,
            by(
                "mark@example.com",
                "GET",
                "/me/artifacts?archived=false",
                None,
            ),
        )
        .await;
        assert!(listed["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_version_can_be_read_and_restored() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Plan", "first").await;
        f.store
            .revise(
                MARK,
                id,
                NewRevision {
                    content: json!({ "markdown": "second" }),
                    reason: "Rewrote it".into(),
                    author: Author::Claude,
                    summary: None,
                },
            )
            .await
            .unwrap();
        let mark = "mark@example.com";
        let (_, one) = send(
            &f.app,
            by(
                mark,
                "GET",
                &format!("/me/artifacts/{id}/revisions/1"),
                None,
            ),
        )
        .await;
        assert_eq!(one["content"]["markdown"], "first");
        let (status, body) = send(
            &f.app,
            by(mark, "POST", &format!("/me/artifacts/{id}/restore/1"), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["content"]["markdown"], "first");
        assert_eq!(body["revisions"][0]["reason"], "Restored version 1");
        assert_eq!(body["revisions"].as_array().unwrap().len(), 3);
        let (status, _) = send(
            &f.app,
            by(mark, "POST", &format!("/me/artifacts/{id}/restore/9"), None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn retry_goes_to_the_binary_for_the_caller() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Plan", "first").await;
        let (status, _) = send(
            &f.app,
            by(
                "mark@example.com",
                "POST",
                &format!("/me/artifacts/{id}/retry"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(f.jobs.retried.lock().unwrap()[0], (MARK.to_string(), id));
    }

    #[tokio::test]
    async fn deleting_says_so_on_the_stream() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Plan", "first").await;
        let mut events = f.bus.subscribe();
        let (status, _) = send(
            &f.app,
            by(
                "mark@example.com",
                "DELETE",
                &format!("/me/artifacts/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(f.store.get(MARK, id).await.unwrap().is_none());
        match events.try_recv().unwrap() {
            Event::ArtifactChanged { status, .. } => assert_eq!(status, "deleted"),
            other => panic!("expected an artifact event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn without_a_database_it_says_so() {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        );
        let app = crate::server::router(state);
        let req = Request::builder()
            .uri("/me/artifacts")
            .body(Body::empty())
            .unwrap();
        let (status, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

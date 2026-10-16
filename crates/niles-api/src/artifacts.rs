//! The signed-in person's artifacts: the Saved tab of Niles's panel.
//! See ARCHITECTURE.md § Artifacts (per person).
//!
//! Every route acts for the caller, by the key their chat is kept
//! under, and never for a person named in the request: another
//! person's id is a 404, the same as one that does not exist.
//!
//! An artifact's pictures are served from here and nowhere else: the
//! copies Niles keeps, by id, to the person they belong to
//! (`GET /me/artifacts/{id}/images/{image}`).
//!
//! And the dashboard's card for today's run, `GET /me/training/today`:
//! the caller's current training plan read for today, by the same
//! [`plan_day`] that `training_today` answers from.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_artifacts::plan_day::{self, PlanDay};
use niles_artifacts::{
    Artifact, ArtifactStore, Author, Changes, Filter, Image, NewRevision, PageChange, Revision,
    RevisionMeta,
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
    /// The HTML of the page it is drawn with, for the app's sealed frame;
    /// `None` for the kind's own view.
    pub page_html: Option<String>,
    /// Newest first.
    pub revisions: Vec<RevisionMeta>,
    /// The pictures it keeps, oldest first, for its views and its page
    /// to fetch by id.
    pub images: Vec<Image>,
    /// The house's today, YYYY-MM-DD: the day a page counts from.
    pub today: String,
}

async fn page_html(
    store: &dyn ArtifactStore,
    person: &str,
    id: i64,
    page: Option<i64>,
) -> Result<Option<String>, Failure> {
    match page {
        Some(page) => store.page(person, id, page).await.map_err(storage),
        None => Ok(None),
    }
}

async fn detail(
    state: &AppState,
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
    let page_html = page_html(store, person, artifact.id, artifact.page).await?;
    let revisions = store
        .revisions(person, artifact.id)
        .await
        .map_err(storage)?;
    let images = store.images(person, artifact.id).await.map_err(storage)?;
    Ok(Detail {
        artifact,
        content,
        page_html,
        revisions,
        images,
        today: house_today(state).to_string(),
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
    Ok(Json(
        detail(&state, store.as_ref(), &person, artifact).await?,
    ))
}

/// A version as it was, with the page it was drawn with then.
#[derive(Debug, Serialize)]
pub struct RevisionDetail {
    #[serde(flatten)]
    pub revision: Revision,
    pub page_html: Option<String>,
}

/// `GET /me/artifacts/{id}/revisions/{n}`
pub async fn revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, n)): Path<(i64, i32)>,
) -> Result<Json<RevisionDetail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let revision = store
        .revision(&person, id, n)
        .await
        .map_err(storage)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such version"))?;
    let page_html = page_html(store.as_ref(), &person, id, revision.page).await?;
    Ok(Json(RevisionDetail {
        revision,
        page_html,
    }))
}

/// Each field: absent leaves it. `content` saves a version by the
/// person, with `reason`, drawn with the page it has; `remove_page`
/// saves one without it, back to the kind's own view (a restore brings
/// it back).
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
    #[serde(default)]
    pub remove_page: Option<bool>,
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
                    page: PageChange::Keep,
                },
            )
            .await
            .map_err(storage)?
            .ok_or_else(not_found)?;
    }
    if body.remove_page == Some(true) && artifact.page.is_some() {
        artifact = store
            .set_page(
                &person,
                id,
                PageChange::Remove,
                "Took the page away".into(),
                Author::Person,
            )
            .await
            .map_err(storage)?
            .ok_or_else(not_found)?;
    }
    announce(&state.event_bus, &artifact);
    Ok(Json(
        detail(&state, store.as_ref(), &person, artifact).await?,
    ))
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
    Ok(Json(
        detail(&state, store.as_ref(), &person, artifact).await?,
    ))
}

/// What a kept picture may be served as: what Niles writes them as.
const IMAGE_TYPES: [&str; 2] = ["image/jpeg", "image/png"];

/// `GET /me/artifacts/{id}/images/{image}`
///
/// One of the caller's pictures, as Niles keeps it. Its id is never
/// reused for other bytes, so the browser keeps it as long as it likes;
/// the headers keep it a picture, whatever it is asked to be: not
/// sniffed, nothing in it run or loaded if it is opened on its own, and
/// not to be shown by another site.
pub async fn image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, image)): Path<(i64, i64)>,
) -> Result<Response, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let kept = store
        .image(&person, id, image)
        .await
        .map_err(storage)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such picture"))?;
    let mime = IMAGE_TYPES
        .into_iter()
        .find(|t| *t == kept.image.mime)
        .ok_or_else(|| failed(StatusCode::BAD_GATEWAY, "not a picture Niles serves"))?;
    let mut response = kept.bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    Ok(response)
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

/// The caller's training plan on one day: `plan` is `None` when they
/// have none with a race still to come, which the card shows as no card.
#[derive(Debug, Serialize)]
pub struct TrainingToday {
    pub plan: Option<PlanToday>,
}

/// Sessions are as the plan keeps them — km, per-km paces, `actual`
/// once run — for the app to show as its plan view does.
#[derive(Debug, Serialize)]
pub struct PlanToday {
    pub id: i64,
    pub title: String,
    /// The plan's goal: race, distance_km, date, target_time.
    pub race: Value,
    /// The house's today, YYYY-MM-DD: what the card counts from and
    /// marks a session done on.
    pub today: String,
    pub days_to_go: i64,
    pub sessions: Vec<Value>,
    pub next: Option<Value>,
    pub week: Option<WeekToday>,
    /// The coach's newest note, `{date, note}`.
    pub update: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct WeekToday {
    pub focus: Option<String>,
    pub planned_km: f64,
    pub run_km: f64,
    pub sessions_left: usize,
}

/// Today in the house's time zone; UTC's for a Niles without a config.
fn house_today(state: &AppState) -> chrono::NaiveDate {
    let tz = state
        .config
        .as_ref()
        .and_then(|c| c.current().home.timezone.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::UTC);
    chrono::Utc::now().with_timezone(&tz).date_naive()
}

/// `GET /me/training/today`
pub async fn training_today(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TrainingToday>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    Ok(Json(
        plan_today(store.as_ref(), &person, house_today(&state)).await?,
    ))
}

async fn plan_today(
    store: &dyn ArtifactStore,
    person: &str,
    today: chrono::NaiveDate,
) -> Result<TrainingToday, Failure> {
    let Some((plan, content)) = plan_day::current_plan(store, person, today)
        .await
        .map_err(storage)?
    else {
        return Ok(TrainingToday { plan: None });
    };
    let d = PlanDay::of(&content, today, today);
    let plan = PlanToday {
        id: plan.id,
        title: plan.title,
        race: content["goal"].clone(),
        today: today.to_string(),
        days_to_go: d.days_to_go,
        sessions: d.sessions.into_iter().cloned().collect(),
        next: d.next.cloned(),
        week: d.week.map(|w| WeekToday {
            focus: w.focus.map(str::to_string),
            planned_km: w.planned_km,
            run_km: w.run_km,
            sessions_left: w.sessions_left,
        }),
        update: d.update.cloned(),
    };
    Ok(TrainingToday { plan: Some(plan) })
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
                        page: PageChange::Keep,
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
                    page: PageChange::Keep,
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

    async fn with_page(store: &MemoryArtifacts, person: &str) -> i64 {
        let id = make(store, person, "Packing list", "- [ ] passport").await;
        store
            .set_page(
                person,
                id,
                PageChange::New("<h1>Packing</h1>".into()),
                "A page for it".into(),
                Author::Claude,
            )
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn an_artifact_comes_with_its_page_and_a_tick_keeps_it() {
        let f = fixture(true);
        let id = with_page(&f.store, MARK).await;
        let mark = "mark@example.com";
        let (status, body) = send(
            &f.app,
            by(mark, "GET", &format!("/me/artifacts/{id}"), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["page_html"], "<h1>Packing</h1>");
        assert!(body["page"].is_i64());

        let (_, ticked) = send(
            &f.app,
            by(
                mark,
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "content": { "markdown": "- [x] passport" }, "reason": "Page: packed" })),
            ),
        )
        .await;
        assert_eq!(ticked["content"]["markdown"], "- [x] passport");
        assert_eq!(ticked["page_html"], "<h1>Packing</h1>");

        let (_, first) = send(
            &f.app,
            by(
                mark,
                "GET",
                &format!("/me/artifacts/{id}/revisions/1"),
                None,
            ),
        )
        .await;
        assert_eq!(first["page_html"], Value::Null, "version 1 had no page");
        let (_, second) = send(
            &f.app,
            by(
                mark,
                "GET",
                &format!("/me/artifacts/{id}/revisions/2"),
                None,
            ),
        )
        .await;
        assert_eq!(second["page_html"], "<h1>Packing</h1>");
    }

    #[tokio::test]
    async fn the_person_can_take_the_page_away_and_restore_it() {
        let f = fixture(true);
        let id = with_page(&f.store, MARK).await;
        let mark = "mark@example.com";
        let (status, gone) = send(
            &f.app,
            by(
                mark,
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "remove_page": true })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{gone}");
        assert_eq!(gone["page"], Value::Null);
        assert_eq!(gone["page_html"], Value::Null);
        assert_eq!(gone["revisions"][0]["reason"], "Took the page away");
        assert_eq!(gone["content"]["markdown"], "- [ ] passport");

        let (_, back) = send(
            &f.app,
            by(mark, "POST", &format!("/me/artifacts/{id}/restore/2"), None),
        )
        .await;
        assert_eq!(back["page_html"], "<h1>Packing</h1>");
    }

    #[tokio::test]
    async fn another_persons_page_is_not_found() {
        let f = fixture(true);
        let id = with_page(&f.store, MAJSE).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "remove_page": true })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert!(
            f.store
                .get(MAJSE, id)
                .await
                .unwrap()
                .unwrap()
                .page
                .is_some()
        );
    }

    /// A picture kept for artifact `id`, and its bytes.
    async fn picture(store: &MemoryArtifacts, person: &str, id: i64) -> (Image, Vec<u8>) {
        let raw = image_jpeg();
        let prepared = niles_artifacts::images::prepare(&raw).unwrap();
        let bytes = prepared.bytes.clone();
        let new = niles_artifacts::NewImage::new(
            prepared,
            "The Grand Palace",
            Some("J. Doe"),
            Some("https://commons.wikimedia.org/wiki/File:x.jpg"),
        )
        .unwrap();
        let kept = store.add_image(person, id, new).await.unwrap().unwrap();
        (kept, bytes)
    }

    fn image_jpeg() -> Vec<u8> {
        let picture = ::image::RgbImage::from_pixel(6, 4, ::image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        picture
            .write_to(&mut out, ::image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn an_artifact_comes_with_its_pictures_and_each_is_served_as_a_picture() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Trip", "![Palace](niles-image:1)").await;
        let (kept, bytes) = picture(&f.store, MARK, id).await;
        let mark = "mark@example.com";
        let (_, body) = send(
            &f.app,
            by(mark, "GET", &format!("/me/artifacts/{id}"), None),
        )
        .await;
        assert_eq!(body["images"][0]["id"], kept.id);
        assert_eq!(body["images"][0]["alt"], "The Grand Palace");
        assert_eq!(body["images"][0]["credit"], "J. Doe");
        assert_eq!(body["images"][0]["width"], 6);
        assert!(body["images"][0].get("bytes").is_none());

        let res = f
            .app
            .clone()
            .oneshot(by(
                mark,
                "GET",
                &format!("/me/artifacts/{id}/images/{}", kept.id),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let h = res.headers().clone();
        assert_eq!(h[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(
            h[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .starts_with("default-src 'none'")
        );
        let cache = h[header::CACHE_CONTROL].to_str().unwrap();
        assert!(cache.starts_with("private, max-age="), "{cache}");
        assert_eq!(h["cross-origin-resource-policy"], "same-origin");
        assert!(h.get(header::SET_COOKIE).is_none());
        let served = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(served.as_ref(), bytes.as_slice());
    }

    #[tokio::test]
    async fn a_picture_is_only_its_owners_and_only_under_its_own_artifact() {
        let f = fixture(true);
        let hers = make(&f.store, MAJSE, "Her trip", "x").await;
        let (her_picture, _) = picture(&f.store, MAJSE, hers).await;
        let his = make(&f.store, MARK, "His trip", "x").await;
        let mark = "mark@example.com";
        for uri in [
            format!("/me/artifacts/{hers}/images/{}", her_picture.id),
            format!("/me/artifacts/{his}/images/{}", her_picture.id),
            format!("/me/artifacts/{his}/images/999"),
        ] {
            let (status, _) = send(&f.app, by(mark, "GET", &uri, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "GET",
                &format!("/me/artifacts/{hers}/images/{}", her_picture.id),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
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
        for uri in ["/me/artifacts", "/me/training/today"] {
            let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
            let (status, _) = send(&app, req).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{uri}");
        }
    }

    /// A plan toward a race 30 days from `today`, with a session today
    /// and a note from the coach; and with `later`, a long run two days
    /// on, which must then still be in today's week.
    async fn plan_for(
        store: &MemoryArtifacts,
        person: &str,
        title: &str,
        today: chrono::NaiveDate,
        later: bool,
    ) -> i64 {
        let day = |n: i64| (today + chrono::Duration::days(n)).to_string();
        let mut sessions = vec![json!({
            "date": day(0), "type": "intervals", "title": "6 × 400 m",
            "distance_km": 7, "pace": "4:45-5:00", "details": "Warm up 10 min",
        })];
        if later {
            sessions.push(json!({
                "date": day(2), "type": "long", "title": "Long run", "distance_km": 12,
            }));
        }
        let content = niles_artifacts::kinds::validate(
            "training_plan",
            &json!({
                "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": day(30) },
                "weeks": [{ "starts": day(0), "focus": "Build", "sessions": sessions }],
                "updates": [{ "date": day(0), "note": "Eased the week a little." }]
            }),
        )
        .unwrap();
        store
            .create(
                person,
                NewArtifact {
                    kind: "training_plan".into(),
                    title: title.into(),
                    summary: String::new(),
                    notify: true,
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: None,
                    conversation_id: None,
                    first: Some(NewRevision {
                        content,
                        reason: "First draft".into(),
                        author: Author::Claude,
                        summary: None,
                        page: PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn today_s_run_is_the_callers_own_plan_read_for_the_house_s_today() {
        let f = fixture(true);
        // The fixture's house is in Copenhagen.
        let today = chrono::Utc::now()
            .with_timezone(&chrono_tz::Europe::Copenhagen)
            .date_naive();
        let id = plan_for(&f.store, MARK, "My 10K", today, false).await;
        plan_for(&f.store, MAJSE, "Her 10K", today, false).await;
        let (status, body) = send(
            &f.app,
            by("mark@example.com", "GET", "/me/training/today", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let plan = &body["plan"];
        assert_eq!(plan["id"], id);
        assert_eq!(plan["title"], "My 10K");
        assert_eq!(plan["race"]["race"], "Hellerup 10K");
        assert_eq!(plan["days_to_go"], 30);
        assert_eq!(plan["sessions"][0]["title"], "6 × 400 m");
        assert_eq!(plan["sessions"][0]["pace"], "4:45-5:00");
        assert_eq!(plan["sessions"][0]["status"], "planned");
        assert_eq!(plan["week"]["focus"], "Build");
        assert_eq!(plan["week"]["run_km"], 0.0);
        assert_eq!(plan["update"]["note"], "Eased the week a little.");
    }

    #[tokio::test]
    async fn without_a_plan_still_to_run_the_plan_is_null() {
        let f = fixture(true);
        make(&f.store, MARK, "Packing list", "- [ ] passport").await;
        let (status, body) = send(
            &f.app,
            by("mark@example.com", "GET", "/me/training/today", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "plan": null }));
    }

    #[tokio::test]
    async fn the_day_is_read_with_its_week_and_the_next_session() {
        let store = MemoryArtifacts::new();
        // A Wednesday: Friday is in the same week.
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        plan_for(&store, MARK, "My 10K", today, true).await;
        let out = plan_today(&store, MARK, today).await.unwrap();
        let plan = out.plan.unwrap();
        assert_eq!(plan.today, "2026-10-14");
        assert_eq!(plan.next.unwrap()["title"], "Long run");
        let week = plan.week.unwrap();
        assert_eq!((week.planned_km, week.run_km), (19.0, 0.0));
        assert_eq!(week.sessions_left, 2);
        let after = chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        assert!(
            plan_today(&store, MARK, after)
                .await
                .unwrap()
                .plan
                .is_none()
        );
    }
}

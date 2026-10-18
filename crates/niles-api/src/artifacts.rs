//! The signed-in person's artifacts: the Saved tab of Niles's panel.
//!
//! Every route acts for the caller, by the key their chat is kept
//! under, and never for a person named in the request. What they may do
//! with an artifact is what [`niles_artifacts::open`] says they have of
//! it: theirs, shared with them to edit, or to view. One they have
//! nothing of is a 404, the same as one that does not exist; one they
//! have but may not do this with is a 403 saying why.
//!
//! An artifact's pictures are served from here and nowhere else: the
//! copies Niles keeps, by id, to the people who can open it
//! (`GET /me/artifacts/{id}/images/{image}`).
//!
//! Its owner shares it from here too (`/me/artifacts/{id}/shares`),
//! with anyone else in the household ([`crate::household`]).
//!
//! And the dashboard's card for today's run, `GET /me/training/today`:
//! the caller's current training plan read for today, by the same
//! [`plan_day`] that `training_today` answers from. Their own only: a
//! plan shared with them follows its owner's runs, not theirs.

use crate::household::{self, Member};
use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_artifacts::plan_day::{self, PlanDay};
use niles_artifacts::{
    Access, Artifact, ArtifactStore, Author, Changes, Filter, Image, Level, NewRevision, Opened,
    Operation, PageChange, Revision, RevisionMeta,
};
use niles_core::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// What only the binary can do for an artifact: run its job again, say
/// which assistant would, and tell someone it was shared with them.
#[async_trait]
pub trait Jobs: Send + Sync {
    /// `claude` or `chatgpt`, whichever would make an artifact now;
    /// `None` when neither is on and able to run.
    fn assistant(&self) -> Option<&'static str>;

    /// Start a failed job again, on the same artifact with the same
    /// brief, as `person`: its owner, or someone it is shared with to
    /// edit, whose job it then is.
    async fn retry(&self, person: &str, id: i64) -> Result<Artifact, Refused>;

    /// Tell `person`, on the devices where they turned notifications
    /// on, that `by` (its owner's name) shared `artifact` with them.
    async fn shared(&self, person: &str, artifact: &Artifact, by: &str);
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
    /// It is shared with them to read only.
    ReadOnly(String),
    Storage(String),
}

type Failure = (StatusCode, Json<Value>);

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, Json(json!({ "error": message.into() })))
}

fn not_found() -> Failure {
    failed(StatusCode::NOT_FOUND, "no such artifact")
}

/// Why someone who can open an artifact may not do this with it.
fn forbidden(access: Access) -> Failure {
    failed(
        StatusCode::FORBIDDEN,
        match access {
            Access::View => "it is shared with you to read only",
            _ => "only its owner can do that",
        },
    )
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

/// Artifact `id` as `person` has it, when they may do every one of
/// `ops` with it.
async fn opened(
    store: &dyn ArtifactStore,
    person: &str,
    id: i64,
    ops: &[Operation],
) -> Result<Opened, Failure> {
    let opened = niles_artifacts::open(store, person, id)
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    if ops.iter().all(|op| opened.allows(*op)) {
        Ok(opened)
    } else {
        Err(forbidden(opened.access))
    }
}

async fn members(state: &AppState) -> Vec<Member> {
    household::household(state.config.as_deref(), state.voices.as_deref()).await
}

/// Tell the open pages of everyone who has it, its owner and whoever it
/// is shared with, so another tab or phone, or another person, shows it
/// too.
pub fn announce(bus: &niles_core::EventBus, artifact: &Artifact) {
    for person in std::iter::once(&artifact.person).chain(&artifact.shared_with) {
        bus.publish(Event::ArtifactChanged {
            person: person.clone(),
            id: artifact.id,
            kind: artifact.kind.clone(),
            title: artifact.title.clone(),
            status: artifact.status.as_str().into(),
            status_text: artifact.status_text.clone(),
        });
    }
}

/// Tell `person`'s open pages it is gone for them: deleted, or no longer
/// shared with them.
pub fn gone(bus: &niles_core::EventBus, artifact: &Artifact, person: &str) {
    bus.publish(Event::ArtifactChanged {
        person: person.to_string(),
        id: artifact.id,
        kind: artifact.kind.clone(),
        title: artifact.title.clone(),
        status: "deleted".into(),
        status_text: None,
    });
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
}

/// What the caller has of an artifact, and whose it is.
#[derive(Debug, Serialize)]
pub struct Sharing {
    pub access: Access,
    /// Whose it is, by name, when it is not the caller's; `None` for
    /// their own, and for someone no longer in the household.
    pub owner: Option<String>,
    /// For its owner, how many people it is shared with; 0 for anyone
    /// else, who is not told who else has it.
    pub shared_with: usize,
    /// Who saved the version shown, by name, when it was someone else.
    pub changed_by: Option<String>,
}

fn sharing(opened: &Opened, person: &str, members: &[Member]) -> Sharing {
    let a = &opened.artifact;
    let mine = opened.access == Access::Owner;
    Sharing {
        access: opened.access,
        owner: (!mine)
            .then(|| household::name_of(members, &a.person))
            .flatten()
            .map(str::to_string),
        shared_with: if mine { a.shared_with.len() } else { 0 },
        changed_by: a
            .changed_by
            .as_deref()
            .filter(|by| *by != person)
            .and_then(|by| household::name_of(members, by))
            .map(str::to_string),
    }
}

/// An artifact in the caller's list.
#[derive(Debug, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub artifact: Artifact,
    #[serde(flatten)]
    pub sharing: Sharing,
}

#[derive(Debug, Serialize)]
pub struct List {
    /// Their own and those shared with them, together: pinned first (by
    /// their own pins), then the most recently changed.
    pub artifacts: Vec<Listed>,
    /// Which assistant makes them now; `None` is why nothing new will be
    /// made, which the Saved tab explains.
    pub assistant: Option<&'static str>,
}

/// `GET /me/artifacts`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<List>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let filter = Filter {
        pinned: query.pinned,
        archived: query.archived,
    };
    let own = store.list(&person, filter).await.map_err(storage)?;
    let shared = store.shared_with(&person, filter).await.map_err(storage)?;
    let members = members(&state).await;
    let mut artifacts: Vec<Listed> = own
        .into_iter()
        .map(|artifact| Opened {
            artifact,
            access: Access::Owner,
        })
        .chain(shared.into_iter().map(|(artifact, level)| Opened {
            artifact,
            access: level.into(),
        }))
        .map(|opened| Listed {
            sharing: sharing(&opened, &person, &members),
            artifact: opened.artifact,
        })
        .collect();
    artifacts.sort_by(|a, b| {
        let (a, b) = (&a.artifact, &b.artifact);
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_at.cmp(&a.updated_at))
            .then(b.id.cmp(&a.id))
    });
    Ok(Json(List {
        artifacts,
        assistant: state.artifact_jobs.as_ref().and_then(|j| j.assistant()),
    }))
}

/// A version in the list of versions.
#[derive(Debug, Serialize)]
pub struct RevisionView {
    #[serde(flatten)]
    pub meta: RevisionMeta,
    /// Whose change it was, by name, when it was not the caller's.
    pub by: Option<String>,
}

fn by_name(by: Option<&str>, person: &str, members: &[Member]) -> Option<String> {
    by.filter(|by| *by != person)
        .and_then(|by| household::name_of(members, by))
        .map(str::to_string)
}

#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub artifact: Artifact,
    #[serde(flatten)]
    pub sharing: Sharing,
    /// The current version's content; `None` before there is one.
    pub content: Option<Value>,
    /// The HTML of the page it is drawn with, for the app's sealed frame;
    /// `None` for the kind's own view.
    pub page_html: Option<String>,
    /// Newest first. Only the current one for someone it is shared with
    /// to read: what it was before is its owner's.
    pub revisions: Vec<RevisionView>,
    /// The pictures it keeps, oldest first, for its views and its page
    /// to fetch by id.
    pub images: Vec<Image>,
    /// The house's today, YYYY-MM-DD: the day a page counts from.
    pub today: String,
}

async fn page_html(
    store: &dyn ArtifactStore,
    owner: &str,
    id: i64,
    page: Option<i64>,
) -> Result<Option<String>, Failure> {
    match page {
        Some(page) => store.page(owner, id, page).await.map_err(storage),
        None => Ok(None),
    }
}

async fn detail(
    state: &AppState,
    store: &dyn ArtifactStore,
    person: &str,
    opened: Opened,
) -> Result<Detail, Failure> {
    let owner = opened.owner().to_string();
    let artifact = &opened.artifact;
    let content = match artifact.current_revision {
        0 => None,
        n => store
            .revision(&owner, artifact.id, n)
            .await
            .map_err(storage)?
            .map(|r| r.content),
    };
    let page_html = page_html(store, &owner, artifact.id, artifact.page).await?;
    let members = members(state).await;
    let revisions = store
        .revisions(&owner, artifact.id)
        .await
        .map_err(storage)?
        .into_iter()
        .filter(|r| opened.allows(Operation::History) || r.n == artifact.current_revision)
        .map(|meta| RevisionView {
            by: by_name(meta.by.as_deref(), person, &members),
            meta,
        })
        .collect();
    let images = store.images(&owner, artifact.id).await.map_err(storage)?;
    Ok(Detail {
        sharing: sharing(&opened, person, &members),
        artifact: opened.artifact,
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
    let opened = opened(store.as_ref(), &person, id, &[Operation::Read]).await?;
    Ok(Json(detail(&state, store.as_ref(), &person, opened).await?))
}

/// A version as it was, with the page it was drawn with then.
#[derive(Debug, Serialize)]
pub struct RevisionDetail {
    #[serde(flatten)]
    pub revision: Revision,
    /// Whose change it was, by name, when it was not the caller's.
    pub by: Option<String>,
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
    let opened = opened(store.as_ref(), &person, id, &[Operation::Read]).await?;
    let no_such_version = || failed(StatusCode::NOT_FOUND, "no such version");
    if n != opened.artifact.current_revision && !opened.allows(Operation::History) {
        return Err(no_such_version());
    }
    let revision = store
        .revision(opened.owner(), id, n)
        .await
        .map_err(storage)?
        .ok_or_else(no_such_version)?;
    let page_html = page_html(store.as_ref(), opened.owner(), id, revision.page).await?;
    let by = by_name(revision.by.as_deref(), &person, &members(&state).await);
    Ok(Json(RevisionDetail {
        revision,
        by,
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

impl Patch {
    /// What it takes to make every change it asks for.
    fn needs(&self) -> Vec<Operation> {
        [
            (self.title.is_some(), Operation::Edit),
            (self.pinned.is_some(), Operation::Pin),
            (self.archived.is_some(), Operation::Archive),
            (self.notify.is_some(), Operation::Notify),
            (self.content.is_some(), Operation::Edit),
            (self.remove_page == Some(true), Operation::Edit),
        ]
        .into_iter()
        .filter_map(|(asked, op)| asked.then_some(op))
        .collect()
    }
}

/// `PATCH /me/artifacts/{id}`
///
/// A pin is the caller's own: the owner's on the artifact, anyone
/// else's on their share of it, so neither moves the other's list.
pub async fn patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<Patch>,
) -> Result<Json<Detail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let opened = opened(store.as_ref(), &person, id, &body.needs()).await?;
    // A podcast is kept as it was recorded; before anything is written.
    if body.content.is_some() && opened.artifact.kind == niles_artifacts::podcast::ID {
        return Err(failed(
            StatusCode::CONFLICT,
            "a podcast's script is kept as it was recorded; ask for a new episode instead",
        ));
    }
    let owner = opened.owner().to_string();
    let mine = opened.access == Access::Owner;
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
        .map(|c| niles_artifacts::kinds::validate(&opened.artifact.kind, c))
        .transpose()
        .map_err(storage)?;
    let mut artifact = store
        .change(
            &owner,
            id,
            Changes {
                title,
                pinned: body.pinned.filter(|_| mine),
                archived: body.archived,
                notify: body.notify,
            },
        )
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    if let (Some(pinned), false) = (body.pinned, mine)
        && !store
            .pin_shared(&person, id, pinned)
            .await
            .map_err(storage)?
    {
        return Err(not_found());
    }
    if let Some(content) = content {
        let reason = body
            .reason
            .as_deref()
            .map(niles_artifacts::summary)
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "Edited in the app".into());
        artifact = store
            .revise(
                &owner,
                id,
                NewRevision {
                    content,
                    reason,
                    author: Author::Person,
                    by: Some(person.clone()),
                    summary: None,
                    page: PageChange::Keep,
                },
                None,
            )
            .await
            .map_err(storage)?
            .ok_or_else(not_found)?;
    }
    if body.remove_page == Some(true) && artifact.page.is_some() {
        artifact = store
            .set_page(
                &owner,
                id,
                PageChange::Remove,
                "Took the page away".into(),
                Author::Person,
                Some(person.clone()),
            )
            .await
            .map_err(storage)?
            .ok_or_else(not_found)?;
    }
    announce(&state.event_bus, &artifact);
    let now = opened_now(store.as_ref(), &person, id).await?;
    Ok(Json(detail(&state, store.as_ref(), &person, now).await?))
}

/// The artifact as the caller has it after a change, with their own pin.
async fn opened_now(store: &dyn ArtifactStore, person: &str, id: i64) -> Result<Opened, Failure> {
    opened(store, person, id, &[Operation::Read]).await
}

/// `POST /me/artifacts/{id}/restore/{n}`
pub async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, n)): Path<(i64, i32)>,
) -> Result<Json<Detail>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let opened = opened(store.as_ref(), &person, id, &[Operation::Edit]).await?;
    if opened.artifact.kind == niles_artifacts::podcast::ID {
        return Err(failed(
            StatusCode::CONFLICT,
            "a podcast is kept as it was recorded, so an older version can't be restored",
        ));
    }
    let artifact = niles_artifacts::restore(store.as_ref(), opened.owner(), id, n, &person)
        .await
        .map_err(storage)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such version"))?;
    announce(&state.event_bus, &artifact);
    let now = opened_now(store.as_ref(), &person, id).await?;
    Ok(Json(detail(&state, store.as_ref(), &person, now).await?))
}

/// What a kept picture may be served as: what Niles writes them as.
const IMAGE_TYPES: [&str; 2] = ["image/jpeg", "image/png"];

/// A podcast's recording is deleted within this, whatever the bucket's
/// mood: the person is waiting on the answer.
const FORGET_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// A time-limited address for a podcast's recording: the bucket is
/// private, and the player fetches it there.
#[derive(Debug, Serialize)]
pub struct EpisodeAudio {
    pub url: String,
    /// When the address stops working, RFC 3339.
    pub expires_at: String,
    /// When the episode itself goes, RFC 3339: `created_at` + the
    /// add-on's keep-days.
    pub kept_until: String,
}

/// `GET /me/artifacts/{id}/audio`
pub async fn audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<(HeaderMap, Json<EpisodeAudio>), Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let opened = opened(store.as_ref(), &person, id, &[Operation::Read]).await?;
    let artifact = &opened.artifact;
    if artifact.kind != niles_artifacts::podcast::ID {
        return Err(failed(StatusCode::NOT_FOUND, "it has no recording"));
    }
    if artifact.status != niles_artifacts::Status::Ready {
        return Err(failed(StatusCode::CONFLICT, "it is not recorded yet"));
    }
    let podcasts = state
        .config
        .as_ref()
        .map(|c| c.current())
        .and_then(|cfg| cfg.integrations.podcasts.clone());
    let bucket = podcasts
        .as_ref()
        .and_then(|p| niles_podcasts::Bucket::from_config(p).ok());
    let (Some(podcasts), Some(bucket)) = (podcasts, bucket) else {
        return Err(failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "Podcasts are not set up, so its recording can't be reached",
        ));
    };
    let now = chrono::Utc::now();
    let valid_for = niles_podcasts::PLAY_FOR;
    let url = bucket.presign_get(&niles_podcasts::episode_key(id), valid_for, now);
    let expires_at = now + chrono::Duration::from_std(valid_for).unwrap_or(chrono::Duration::MAX);
    let kept_until = artifact.created_at + chrono::Duration::days(podcasts.keep_days.max(1) as i64);
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        response_headers,
        Json(EpisodeAudio {
            url,
            expires_at: expires_at.to_rfc3339(),
            kept_until: kept_until.to_rfc3339(),
        }),
    ))
}

/// Delete a podcast's recording before its row. Refused while the
/// bucket can't be reached, so the row stays to try again; only with
/// the add-on gone does the row go alone, its object left to the
/// bucket's lifecycle rule.
async fn forget_recording(state: &AppState, id: i64) -> Result<(), Failure> {
    let podcasts = state
        .config
        .as_ref()
        .map(|c| c.current())
        .and_then(|cfg| cfg.integrations.podcasts.clone());
    let Some(podcasts) = podcasts else {
        tracing::warn!(
            artifact = id,
            "the Podcasts add-on is gone; its recording stays until the bucket's lifecycle \
             rule clears it"
        );
        return Ok(());
    };
    // Only the bucket is needed to delete: a Kokoro address being
    // edited must not leave an object behind its row.
    let bucket = niles_podcasts::Bucket::from_config(&podcasts).map_err(|e| {
        failed(
            StatusCode::BAD_GATEWAY,
            format!("its recording could not be deleted from the bucket, so it was kept: {e}"),
        )
    })?;
    match tokio::time::timeout(
        FORGET_WITHIN,
        niles_podcasts::Storage::delete(&bucket, &niles_podcasts::episode_key(id)),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(failed(
            StatusCode::BAD_GATEWAY,
            format!("its recording could not be deleted from the bucket, so it was kept: {e}"),
        )),
        Err(_) => Err(failed(
            StatusCode::BAD_GATEWAY,
            "its recording could not be deleted from the bucket in time, so it was kept",
        )),
    }
}

/// `GET /me/artifacts/{id}/images/{image}`
///
/// One of the pictures of an artifact the caller can open, as Niles
/// keeps it. Its id is never reused for other bytes, so the browser
/// keeps it as long as it likes; the headers keep it a picture, whatever
/// it is asked to be: not sniffed, nothing in it run or loaded if it is
/// opened on its own, and not to be shown by another site.
pub async fn image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, image)): Path<(i64, i64)>,
) -> Result<Response, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let no_such_picture = || failed(StatusCode::NOT_FOUND, "no such picture");
    let opened = opened(store.as_ref(), &person, id, &[Operation::Read])
        .await
        .map_err(|_| no_such_picture())?;
    let kept = store
        .image(opened.owner(), id, image)
        .await
        .map_err(storage)?
        .ok_or_else(no_such_picture)?;
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
    let store = store(&state)?;
    let jobs = state.artifact_jobs.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance runs no jobs",
        )
    })?;
    let person = crate::chat::person_of(&state, &headers);
    opened(store.as_ref(), &person, id, &[Operation::Edit]).await?;
    match jobs.retry(&person, id).await {
        Ok(artifact) => Ok(Json(artifact)),
        Err(Refused::NotFound) => Err(not_found()),
        Err(Refused::NotRetryable(why)) => Err(failed(StatusCode::CONFLICT, why)),
        Err(Refused::Unavailable(why)) => Err(failed(StatusCode::SERVICE_UNAVAILABLE, why)),
        Err(Refused::ReadOnly(why)) => Err(failed(StatusCode::FORBIDDEN, why)),
        Err(Refused::Storage(why)) => Err(failed(StatusCode::BAD_GATEWAY, why)),
    }
}

/// `DELETE /me/artifacts/{id}`: its owner's alone. It goes from the
/// lists of everyone it was shared with too.
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let opened = opened(store.as_ref(), &person, id, &[Operation::Delete]).await?;
    // A podcast's recording goes first: a row without its object is a
    // thing the sweep can fix; an object without its row is a leak.
    if opened.artifact.kind == niles_artifacts::podcast::ID {
        forget_recording(&state, id).await?;
    }
    if !store.delete(&person, id).await.map_err(storage)? {
        return Err(not_found());
    }
    let artifact = &opened.artifact;
    for who in std::iter::once(&artifact.person).chain(&artifact.shared_with) {
        gone(&state.event_bus, artifact, who);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Someone the owner can share an artifact with, and how it is shared
/// with them now.
#[derive(Debug, Serialize)]
pub struct Sharee {
    pub person: String,
    /// `None` for someone it is still shared with who is no longer in
    /// the household: the app says so, and offers to stop.
    pub name: Option<String>,
    /// `None` when it is not shared with them.
    pub level: Option<Level>,
}

#[derive(Debug, Serialize)]
pub struct Shares {
    /// Everybody else in the household, in the household's order, then
    /// anyone it is shared with who is no longer in it.
    pub people: Vec<Sharee>,
}

async fn shares_of(
    state: &AppState,
    store: &dyn ArtifactStore,
    owner: &str,
    id: i64,
) -> Result<Shares, Failure> {
    let shares = store.shares(owner, id).await.map_err(storage)?;
    let members = members(state).await;
    let level_of = |person: &str| shares.iter().find(|s| s.person == person).map(|s| s.level);
    let mut people: Vec<Sharee> = members
        .iter()
        .filter(|m| m.person != owner)
        .map(|m| Sharee {
            person: m.person.clone(),
            name: Some(m.name.clone()),
            level: level_of(&m.person),
        })
        .collect();
    people.extend(
        shares
            .iter()
            .filter(|s| household::name_of(&members, &s.person).is_none())
            .map(|s| Sharee {
                person: s.person.clone(),
                name: None,
                level: Some(s.level),
            }),
    );
    Ok(Shares { people })
}

/// `GET /me/artifacts/{id}/shares`: its owner's alone.
pub async fn shares(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Shares>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    opened(store.as_ref(), &person, id, &[Operation::Share]).await?;
    Ok(Json(shares_of(&state, store.as_ref(), &person, id).await?))
}

/// Share with one person, at a level, or (`level: null`) stop.
#[derive(Debug, Deserialize)]
pub struct ShareWith {
    pub person: String,
    #[serde(default)]
    pub level: Option<Level>,
}

/// `PUT /me/artifacts/{id}/shares`: its owner's alone.
///
/// Only with someone in the household; stopping also for someone it is
/// shared with who is no longer in it. Someone it is newly shared with
/// is told on their devices; a change of level or a stop is not news to
/// push, and their list follows it on the event stream.
pub async fn share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ShareWith>,
) -> Result<Json<Shares>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    opened(store.as_ref(), &person, id, &[Operation::Share]).await?;
    let before = store.shares(&person, id).await.map_err(storage)?;
    let was = before.iter().find(|s| s.person == body.person);
    let members = members(&state).await;
    let in_household = household::name_of(&members, &body.person).is_some();
    if body.level.is_some() && (!in_household || body.person == person) {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "it can be shared only with someone else in the household",
        ));
    }
    store
        .set_share(&person, id, &body.person, body.level)
        .await
        .map_err(storage)?;
    let now = store
        .get(&person, id)
        .await
        .map_err(storage)?
        .ok_or_else(not_found)?;
    match (was, body.level) {
        (Some(_), None) => gone(&state.event_bus, &now, &body.person),
        (None, Some(_)) => {
            if let Some(jobs) = &state.artifact_jobs {
                let by = household::name_of(&members, &person)
                    .map_or_else(|| household::name_from_key(&person), str::to_string);
                jobs.shared(&body.person, &now, &by).await;
            }
        }
        _ => {}
    }
    announce(&state.event_bus, &now);
    Ok(Json(shares_of(&state, store.as_ref(), &person, id).await?))
}

/// `DELETE /me/artifacts/{id}/shares`: stop sharing it with anyone. Its
/// owner's alone.
pub async fn unshare_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Shares>, Failure> {
    let store = store(&state)?;
    let person = crate::chat::person_of(&state, &headers);
    let opened = opened(store.as_ref(), &person, id, &[Operation::Share]).await?;
    for who in &opened.artifact.shared_with {
        store
            .set_share(&person, id, who, None)
            .await
            .map_err(storage)?;
        gone(&state.event_bus, &opened.artifact, who);
    }
    Ok(Json(shares_of(&state, store.as_ref(), &person, id).await?))
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
        /// Who was told what was shared with them, and by whom.
        told: std::sync::Mutex<Vec<(String, String, String)>>,
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

        async fn shared(&self, person: &str, artifact: &Artifact, by: &str) {
            self.told
                .lock()
                .unwrap()
                .push((person.into(), artifact.title.clone(), by.into()));
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
  {{ email = "anna@example.com" }},
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
                        by: None,
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
                    by: None,
                    summary: None,
                    page: PageChange::Keep,
                },
                None,
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
                None,
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
                        by: None,
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

    const ANNA: &str = "email:anna@example.com";

    async fn share_with(f: &Fixture, id: i64, person: &str, level: Level) {
        assert!(
            f.store
                .set_share(MARK, id, person, Some(level))
                .await
                .unwrap()
        );
    }

    /// His trip, at version 2, with a picture.
    async fn his_trip(f: &Fixture) -> (i64, Image) {
        let id = make(&f.store, MARK, "Thailand trip", "- [ ] passport").await;
        f.store
            .revise(
                MARK,
                id,
                NewRevision {
                    content: json!({ "markdown": "- [ ] passport\n- [ ] charger" }),
                    reason: "Added the charger".into(),
                    author: Author::Claude,
                    by: Some(MARK.into()),
                    summary: None,
                    page: PageChange::Keep,
                },
                None,
            )
            .await
            .unwrap();
        let (picture, _) = picture(&f.store, MARK, id).await;
        (id, picture)
    }

    fn persons(events: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<(String, String)> {
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|e| match e {
                Event::ArtifactChanged { person, status, .. } => Some((person, status)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn the_list_has_what_is_shared_with_the_caller_and_whose_it_is() {
        let f = fixture(true);
        let hers = make(&f.store, MAJSE, "Her notes", "x").await;
        let (his, _) = his_trip(&f).await;
        share_with(&f, his, MAJSE, Level::View).await;
        let (status, body) = send(
            &f.app,
            by("majse@example.com", "GET", "/me/artifacts", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let listed = body["artifacts"].as_array().unwrap();
        assert_eq!(listed.len(), 2);
        let trip = listed.iter().find(|a| a["id"] == his).unwrap();
        assert_eq!(trip["access"], "view");
        assert_eq!(trip["owner"], "Mark");
        assert_eq!(trip["shared_with"], 0, "who else has it is his business");
        assert_eq!(trip["changed_by"], "Mark");
        assert!(trip.get("person").is_none());
        let notes = listed.iter().find(|a| a["id"] == hers).unwrap();
        assert_eq!(notes["access"], "owner");
        assert_eq!(notes["owner"], Value::Null);

        let (_, his_list) =
            send(&f.app, by("mark@example.com", "GET", "/me/artifacts", None)).await;
        assert_eq!(his_list["artifacts"].as_array().unwrap().len(), 1);
        assert_eq!(his_list["artifacts"][0]["access"], "owner");
        assert_eq!(his_list["artifacts"][0]["shared_with"], 1);
        assert_eq!(
            his_list["artifacts"][0]["changed_by"],
            Value::Null,
            "his own change"
        );
    }

    #[tokio::test]
    async fn a_pin_on_a_shared_one_is_the_callers_own() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::View).await;
        let uri = format!("/me/artifacts/{id}");
        let pin = |who: &'static str, pinned: bool| {
            by(who, "PATCH", &uri, Some(json!({ "pinned": pinned })))
        };
        let (status, hers) = send(&f.app, pin("majse@example.com", true)).await;
        assert_eq!(status, StatusCode::OK, "{hers}");
        assert_eq!(hers["pinned"], true);
        assert!(!f.store.get(MARK, id).await.unwrap().unwrap().pinned);

        let (_, his) = send(&f.app, pin("mark@example.com", true)).await;
        assert_eq!(his["pinned"], true);
        let (_, hers) = send(&f.app, pin("majse@example.com", false)).await;
        assert_eq!(hers["pinned"], false);
        assert!(f.store.get(MARK, id).await.unwrap().unwrap().pinned);
        let (_, listed) = send(
            &f.app,
            by(
                "majse@example.com",
                "GET",
                "/me/artifacts?pinned=true",
                None,
            ),
        )
        .await;
        assert!(listed["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shared_to_read_it_opens_but_nothing_in_it_changes() {
        let f = fixture(true);
        let (id, picture) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::View).await;
        let majse = "majse@example.com";
        let (status, body) = send(
            &f.app,
            by(majse, "GET", &format!("/me/artifacts/{id}"), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["access"], "view");
        assert_eq!(body["content"]["markdown"], "- [ ] passport\n- [ ] charger");
        let versions = body["revisions"].as_array().unwrap();
        assert_eq!(versions.len(), 1, "the current one only");
        assert_eq!(versions[0]["n"], 2);
        assert_eq!(versions[0]["by"], "Mark");
        let (status, _) = send(
            &f.app,
            by(
                majse,
                "GET",
                &format!("/me/artifacts/{id}/revisions/2"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &f.app,
            by(
                majse,
                "GET",
                &format!("/me/artifacts/{id}/revisions/1"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(
            &f.app,
            by(
                majse,
                "GET",
                &format!("/me/artifacts/{id}/images/{}", picture.id),
                None,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "its pictures are part of reading it"
        );

        for (method, uri, body) in [
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "content": { "markdown": "- [x] passport" }, "reason": "Packed" })),
            ),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "title": "Mine now" })),
            ),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "remove_page": true })),
            ),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "archived": true })),
            ),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "notify": false })),
            ),
            ("POST", format!("/me/artifacts/{id}/restore/1"), None),
            ("POST", format!("/me/artifacts/{id}/retry"), None),
            ("DELETE", format!("/me/artifacts/{id}"), None),
            ("GET", format!("/me/artifacts/{id}/shares"), None),
            (
                "PUT",
                format!("/me/artifacts/{id}/shares"),
                Some(json!({ "person": ANNA, "level": "view" })),
            ),
            ("DELETE", format!("/me/artifacts/{id}/shares"), None),
        ] {
            let (status, body) = send(&f.app, by(majse, method, &uri, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
        }
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!(
            (a.title.as_str(), a.current_revision, a.archived, a.notify),
            ("Thailand trip", 2, false, true)
        );
        assert!(f.jobs.retried.lock().unwrap().is_empty());
        assert_eq!(f.store.shares(MARK, id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shared_to_edit_it_changes_as_its_owner_would_short_of_owning_it() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::Edit).await;
        let majse = "majse@example.com";
        let uri = format!("/me/artifacts/{id}");
        let (status, body) = send(
            &f.app,
            by(
                majse,
                "PATCH",
                &uri,
                Some(json!({ "content": { "markdown": "- [x] passport\n- [ ] charger" }, "reason": "Packed passport" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["access"], "edit");
        assert_eq!(body["revisions"].as_array().unwrap().len(), 3);
        assert_eq!(body["revisions"][0]["author"], "person");
        assert_eq!(body["revisions"][0]["by"], Value::Null, "her own");
        let r = f.store.revision(MARK, id, 3).await.unwrap().unwrap();
        assert_eq!(r.by.as_deref(), Some(MAJSE));

        let (_, his) = send(&f.app, by("mark@example.com", "GET", &uri, None)).await;
        assert_eq!(his["revisions"][0]["by"], "Majse");
        assert_eq!(his["changed_by"], "Majse");

        let (status, _) = send(
            &f.app,
            by(
                majse,
                "GET",
                &format!("/me/artifacts/{id}/revisions/1"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "an editor reads the versions");
        let (status, restored) = send(
            &f.app,
            by(
                majse,
                "POST",
                &format!("/me/artifacts/{id}/restore/1"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{restored}");
        assert_eq!(restored["content"]["markdown"], "- [ ] passport");
        let r = f.store.revision(MARK, id, 4).await.unwrap().unwrap();
        assert_eq!((r.author, r.by.as_deref()), (Author::Person, Some(MAJSE)));
        let (status, _) = send(
            &f.app,
            by(
                majse,
                "PATCH",
                &uri,
                Some(json!({ "title": "Our Thailand trip" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for (method, uri, body) in [
            ("PATCH", uri.clone(), Some(json!({ "archived": true }))),
            ("PATCH", uri.clone(), Some(json!({ "notify": false }))),
            ("DELETE", uri.clone(), None),
            ("GET", format!("{uri}/shares"), None),
            (
                "PUT",
                format!("{uri}/shares"),
                Some(json!({ "person": ANNA, "level": "edit" })),
            ),
        ] {
            let (status, body) = send(&f.app, by(majse, method, &uri, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {body}");
        }
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert!(!a.archived && a.notify);
        assert_eq!(a.shared_with, vec![MAJSE]);
    }

    #[tokio::test]
    async fn someone_it_is_not_shared_with_finds_nothing_on_any_route() {
        let f = fixture(true);
        let (id, picture) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::Edit).await;
        for (method, uri, body) in [
            ("GET", format!("/me/artifacts/{id}"), None),
            ("GET", format!("/me/artifacts/{id}/revisions/2"), None),
            (
                "GET",
                format!("/me/artifacts/{id}/images/{}", picture.id),
                None,
            ),
            (
                "PATCH",
                format!("/me/artifacts/{id}"),
                Some(json!({ "pinned": true })),
            ),
            ("POST", format!("/me/artifacts/{id}/restore/1"), None),
            ("POST", format!("/me/artifacts/{id}/retry"), None),
            ("DELETE", format!("/me/artifacts/{id}"), None),
            ("GET", format!("/me/artifacts/{id}/shares"), None),
            (
                "PUT",
                format!("/me/artifacts/{id}/shares"),
                Some(json!({ "person": ANNA, "level": "edit" })),
            ),
            ("DELETE", format!("/me/artifacts/{id}/shares"), None),
        ] {
            let (status, _) = send(&f.app, by("anna@example.com", method, &uri, body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
        let (_, listed) = send(&f.app, by("anna@example.com", "GET", "/me/artifacts", None)).await;
        assert!(listed["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_not_shared_with_them_answers_exactly_as_one_that_is_not_there() {
        let f = fixture(true);
        let (his, picture) = his_trip(&f).await;
        share_with(&f, his, MAJSE, Level::Edit).await;
        let missing = his + 100;
        let routes = |id: i64| {
            vec![
                ("GET", format!("/me/artifacts/{id}"), None),
                ("GET", format!("/me/artifacts/{id}/revisions/1"), None),
                (
                    "GET",
                    format!("/me/artifacts/{id}/images/{}", picture.id),
                    None,
                ),
                (
                    "PATCH",
                    format!("/me/artifacts/{id}"),
                    Some(json!({ "title": "x" })),
                ),
                ("POST", format!("/me/artifacts/{id}/restore/1"), None),
                ("POST", format!("/me/artifacts/{id}/retry"), None),
                ("DELETE", format!("/me/artifacts/{id}"), None),
                ("GET", format!("/me/artifacts/{id}/shares"), None),
                (
                    "PUT",
                    format!("/me/artifacts/{id}/shares"),
                    Some(json!({ "person": MAJSE, "level": "view" })),
                ),
                ("DELETE", format!("/me/artifacts/{id}/shares"), None),
            ]
        };
        let mut events = f.bus.subscribe();
        for ((method, uri, body), (_, gone, _)) in routes(his).into_iter().zip(routes(missing)) {
            let anna = "anna@example.com";
            let shared = send(&f.app, by(anna, method, &uri, body.clone())).await;
            let absent = send(&f.app, by(anna, method, &gone, body)).await;
            assert_eq!(shared, absent, "{method} {uri}");
        }
        assert!(
            persons(&mut events).is_empty(),
            "nothing happened, to anyone"
        );
        let (_, listed) = send(&f.app, by("anna@example.com", "GET", "/me/artifacts", None)).await;
        assert_eq!(listed["artifacts"], json!([]));
        let (_, today) = send(
            &f.app,
            by("anna@example.com", "GET", "/me/training/today", None),
        )
        .await;
        assert_eq!(today, json!({ "plan": null }));
    }

    #[tokio::test]
    async fn a_change_is_heard_only_by_those_who_have_it() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::View).await;
        let mut events = f.bus.subscribe();
        send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "title": "Thailand, March" })),
            ),
        )
        .await;
        let heard = persons(&mut events);
        assert!(!heard.iter().any(|(p, _)| p == ANNA), "{heard:?}");
        assert_eq!(heard.len(), 2);
    }

    #[tokio::test]
    async fn the_owner_shares_with_the_household_and_they_are_told_once() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        let mark = "mark@example.com";
        let uri = format!("/me/artifacts/{id}/shares");
        let (status, body) = send(&f.app, by(mark, "GET", &uri, None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["people"],
            json!([
                { "person": MAJSE, "name": "Majse", "level": null },
                { "person": ANNA, "name": "Anna", "level": null },
            ]),
            "the household but him"
        );

        let mut events = f.bus.subscribe();
        let (status, body) = send(
            &f.app,
            by(
                mark,
                "PUT",
                &uri,
                Some(json!({ "person": MAJSE, "level": "edit" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["people"][0]["level"], "edit");
        assert_eq!(
            *f.jobs.told.lock().unwrap(),
            vec![(
                MAJSE.to_string(),
                "Thailand trip".to_string(),
                "Mark".to_string()
            )]
        );
        let heard = persons(&mut events);
        assert!(heard.contains(&(MAJSE.into(), "ready".into())), "{heard:?}");
        assert!(heard.contains(&(MARK.into(), "ready".into())), "{heard:?}");

        let (status, _) = send(
            &f.app,
            by(
                mark,
                "PUT",
                &uri,
                Some(json!({ "person": MAJSE, "level": "view" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            f.jobs.told.lock().unwrap().len(),
            1,
            "a change of level is no news"
        );
        for outsider in ["person:stranger", MARK] {
            let (status, _) = send(
                &f.app,
                by(
                    mark,
                    "PUT",
                    &uri,
                    Some(json!({ "person": outsider, "level": "view" })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{outsider}");
        }

        persons(&mut events);
        let (status, body) = send(
            &f.app,
            by(
                mark,
                "PUT",
                &uri,
                Some(json!({ "person": MAJSE, "level": null })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["people"][0]["level"], Value::Null);
        assert!(persons(&mut events).contains(&(MAJSE.into(), "deleted".into())));
        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "GET",
                &format!("/me/artifacts/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn someone_no_longer_in_the_household_shows_as_nobody_and_can_be_let_go() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        share_with(&f, id, "person:gone", Level::View).await;
        let mark = "mark@example.com";
        let uri = format!("/me/artifacts/{id}/shares");
        let (_, body) = send(&f.app, by(mark, "GET", &uri, None)).await;
        assert_eq!(
            body["people"][2],
            json!({ "person": "person:gone", "name": null, "level": "view" })
        );
        let (status, _) = send(
            &f.app,
            by(
                mark,
                "PUT",
                &uri,
                Some(json!({ "person": "person:gone", "level": "edit" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, body) = send(
            &f.app,
            by(
                mark,
                "PUT",
                &uri,
                Some(json!({ "person": "person:gone", "level": null })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["people"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_change_reaches_the_owner_and_everyone_it_is_shared_with() {
        let f = fixture(true);
        let (id, _) = his_trip(&f).await;
        share_with(&f, id, MAJSE, Level::View).await;
        share_with(&f, id, ANNA, Level::Edit).await;
        let mut events = f.bus.subscribe();
        let (status, _) = send(
            &f.app,
            by(
                "anna@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({ "content": { "markdown": "- [x] passport" } })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut heard: Vec<String> = persons(&mut events).into_iter().map(|(p, _)| p).collect();
        heard.sort();
        assert_eq!(heard, vec![ANNA, MAJSE, MARK]);
    }

    #[tokio::test]
    async fn deleting_or_unsharing_all_takes_it_from_everyone_it_was_shared_with() {
        let f = fixture(true);
        let (kept, _) = his_trip(&f).await;
        let (deleted, _) = his_trip(&f).await;
        for id in [kept, deleted] {
            share_with(&f, id, MAJSE, Level::Edit).await;
        }
        let mark = "mark@example.com";
        let mut events = f.bus.subscribe();
        let (status, _) = send(
            &f.app,
            by(
                mark,
                "DELETE",
                &format!("/me/artifacts/{kept}/shares"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(persons(&mut events), vec![(MAJSE.into(), "deleted".into())]);
        let (status, _) = send(
            &f.app,
            by(mark, "DELETE", &format!("/me/artifacts/{deleted}"), None),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            persons(&mut events),
            vec![
                (MARK.into(), "deleted".into()),
                (MAJSE.into(), "deleted".into())
            ]
        );
        let (_, listed) = send(
            &f.app,
            by("majse@example.com", "GET", "/me/artifacts", None),
        )
        .await;
        assert!(listed["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_plan_shared_with_the_caller_is_not_their_run_today() {
        let f = fixture(true);
        let today = chrono::Utc::now()
            .with_timezone(&chrono_tz::Europe::Copenhagen)
            .date_naive();
        let id = plan_for(&f.store, MARK, "His 10K", today, false).await;
        share_with(&f, id, MAJSE, Level::Edit).await;
        let (status, body) = send(
            &f.app,
            by("majse@example.com", "GET", "/me/training/today", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "plan": null }));
    }

    /// A podcast the fixture's owner can be asked about: valid script,
    /// a status, and the config pointing at the wiremock bucket.
    async fn episode(f: &Fixture, status: Status, working_text: Option<&str>) -> i64 {
        let script = json!({
            "topic": "The Øresund bridge",
            "minutes": 5,
            "hosts": ["Maya", "Leo"],
            "lines": [
                { "host": "Maya", "text": "Twenty years ago this month, the first car drove across." },
                { "host": "Leo", "text": "And by car number ten million, it had made one region of two cities." },
            ],
            "sources": [{ "title": "Øresundsbron", "url": "https://www.oresundsbron.com/" }],
        });
        f.store
            .create(
                MARK,
                NewArtifact {
                    kind: niles_artifacts::podcast::ID.into(),
                    title: "The Øresund bridge".into(),
                    summary: "a summary".into(),
                    notify: true,
                    status,
                    status_text: working_text.map(str::to_string),
                    job_id: None,
                    brief: None,
                    conversation_id: None,
                    first: Some(niles_artifacts::NewRevision {
                        content: script,
                        reason: "First draft".into(),
                        author: Author::Chatgpt,
                        by: Some(MARK.into()),
                        summary: None,
                        page: PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
            .id
    }

    /// The config store the fixture built, with `[integrations.podcasts]`
    /// added, pointing at the mock bucket.
    async fn with_podcasts_config(f: Fixture, server: &wiremock::MockServer) -> Fixture {
        // SAFETY: test-only variables nothing else reads.
        unsafe {
            std::env::set_var("NILES_TEST_ARTIFACTS_PODCASTS_AKID", "AKIDEXAMPLE");
            std::env::set_var(
                "NILES_TEST_ARTIFACTS_PODCASTS_SECRET",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            );
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
  {{ email = "anna@example.com" }},
]
[integrations.podcasts]
kokoro_url = "http://kokoro:8880"
endpoint = "{}"
bucket = "episodes"
region = "auto"
access_key_id_env = "NILES_TEST_ARTIFACTS_PODCASTS_AKID"
secret_access_key_env = "NILES_TEST_ARTIFACTS_PODCASTS_SECRET"
keep_days = 14
"#,
            crate::config_tests::base_toml(),
            server.uri()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            f.bus.clone(),
        )
        .with_config_store(Some(config))
        .with_artifacts(Some(f.store.clone() as Arc<dyn ArtifactStore>), None);
        Fixture {
            app: crate::server::router(state),
            store: f.store,
            jobs: f.jobs,
            bus: f.bus,
        }
    }

    async fn bucket() -> wiremock::MockServer {
        wiremock::MockServer::start().await
    }

    #[tokio::test]
    async fn the_owner_gets_a_presigned_address_for_a_ready_episode() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let server = bucket().await;
        let f = with_podcasts_config(f, &server).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let url = body["url"].as_str().expect("a url");
        assert!(
            url.starts_with(&format!(
                "{}/episodes/podcasts/{id}.mp3?X-Amz-Algorithm=AWS4-HMAC-SHA256",
                server.uri()
            )),
            "unexpected: {url}"
        );
        assert!(
            url.contains("%2Fauto%2Fs3%2Faws4_request"),
            "unexpected: {url}"
        );
        assert!(url.contains("X-Amz-Expires=14400"), "unexpected: {url}");
        for key in ["expires_at", "kept_until"] {
            let at = body[key].as_str().expect("a timestamp");
            chrono::DateTime::parse_from_rfc3339(at).expect("RFC 3339");
        }
    }

    #[tokio::test]
    async fn someone_it_is_shared_with_to_view_gets_one_too() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        share_with(&f, id, MAJSE, Level::View).await;
        let server = bucket().await;
        let f = with_podcasts_config(f, &server).await;
        let (status, body) = send(
            &f.app,
            by(
                "majse@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body["url"]
                .as_str()
                .expect("a url")
                .contains("X-Amz-Signature=")
        );
    }

    #[tokio::test]
    async fn a_stranger_gets_nothing() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let server = bucket().await;
        let f = with_podcasts_config(f, &server).await;
        let (status, _) = send(
            &f.app,
            by(
                "anna@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_working_episode_says_it_is_not_recorded_yet() {
        let f = fixture(true);
        let id = episode(
            &f,
            Status::Working,
            Some("Researching “The Øresund bridge”…"),
        )
        .await;
        let server = bucket().await;
        let f = with_podcasts_config(f, &server).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    #[tokio::test]
    async fn a_document_has_no_recording() {
        let f = fixture(true);
        let id = make(&f.store, MARK, "Notes", "# notes").await;
        let server = bucket().await;
        let f = with_podcasts_config(f, &server).await;
        let (status, _) = send(
            &f.app,
            by(
                "mark@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn without_the_add_on_the_recording_cant_be_reached() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "GET",
                &format!("/me/artifacts/{id}/audio"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    }

    #[tokio::test]
    async fn a_podcasts_content_cant_be_patched_and_nothing_changes() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "PATCH",
                &format!("/me/artifacts/{id}"),
                Some(json!({
                    "content": { "topic": "Changed", "minutes": 5, "hosts": ["Maya", "Leo"],
                        "lines": [{ "host": "Maya", "text": "one two three four five" },
                                  { "host": "Leo", "text": "six seven eight nine ten" }],
                        "sources": [{ "title": "T", "url": "https://example.com/" }] },
                    "reason": "tried",
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let current = f
            .store
            .revision(
                MARK,
                id,
                f.store
                    .get(MARK, id)
                    .await
                    .unwrap()
                    .unwrap()
                    .current_revision,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.content["topic"], json!("The Øresund bridge"));
    }

    #[tokio::test]
    async fn a_podcast_cant_be_restored() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "POST",
                &format!("/me/artifacts/{id}/restore/1"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    #[tokio::test]
    async fn deleting_a_podcast_reaches_the_bucket_before_the_row() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let server = bucket().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path(format!(
                "/episodes/podcasts/{id}.mp3"
            )))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let f = with_podcasts_config(f, &server).await;
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
    }

    #[tokio::test]
    async fn a_bucket_refusal_keeps_the_row_and_says_so() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
        let server = bucket().await;
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path(format!(
                "/episodes/podcasts/{id}.mp3"
            )))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let f = with_podcasts_config(f, &server).await;
        let (status, body) = send(
            &f.app,
            by(
                "mark@example.com",
                "DELETE",
                &format!("/me/artifacts/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert!(
            body["error"]
                .as_str()
                .expect("an error")
                .contains("its recording could not be deleted"),
            "{body}"
        );
        assert!(f.store.get(MARK, id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn deleting_an_episode_with_incomplete_settings_does_not_orphan_its_audio() {
        for (patch, bucket_deletes, expected) in [
            (json!({ "kokoro_url": "" }), 1, StatusCode::NO_CONTENT),
            (json!({ "bucket": "" }), 0, StatusCode::BAD_GATEWAY),
        ] {
            let f = fixture(true);
            let id = episode(&f, Status::Ready, None).await;
            let server = bucket().await;
            wiremock::Mock::given(wiremock::matchers::method("DELETE"))
                .and(wiremock::matchers::path(format!(
                    "/episodes/podcasts/{id}.mp3"
                )))
                .respond_with(wiremock::ResponseTemplate::new(204))
                .expect(bucket_deletes)
                .mount(&server)
                .await;
            let f = with_podcasts_config(f, &server).await;
            let (status, body) = send(
                &f.app,
                by(
                    "mark@example.com",
                    "PATCH",
                    "/config",
                    Some(json!({ "integrations": { "podcasts": patch } })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let (status, body) = send(
                &f.app,
                by(
                    "mark@example.com",
                    "DELETE",
                    &format!("/me/artifacts/{id}"),
                    None,
                ),
            )
            .await;
            assert_eq!(status, expected, "{body}");
            assert_eq!(
                f.store.get(MARK, id).await.unwrap().is_none(),
                expected == StatusCode::NO_CONTENT
            );
        }
    }

    #[tokio::test]
    async fn without_the_add_on_the_row_still_goes() {
        let f = fixture(true);
        let id = episode(&f, Status::Ready, None).await;
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
    }
}

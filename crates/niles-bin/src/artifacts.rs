//! Artifacts: what Claude or ChatGPT makes for one person and keeps for
//! them. See ARCHITECTURE.md § Artifacts (per person).
//!
//! The tools here are offered to the assistant alone (see
//! [`crate::assistant::Assistant::start`]), and act for the asker of
//! the turn, [`approvals::ASKER`]: whose artifacts these are is never an
//! argument. A big one is a job: the artifact is made at once, marked
//! working, and a background run of the same assistant writes it while
//! the chat goes on.
//!
//! A training plan then keeps up with the runs ([`coach`]).
//!
//! Any artifact may have a page: HTML the assistant writes once that
//! draws the data, shown by the app in a sealed frame. Making one is a
//! job of its own on an artifact that is there already
//! (`start_page_job`), and never the coach's: a look after a run changes
//! the data, which the page then draws.
//!
//! An artifact keeps pictures too, Niles's own copies of ones found on
//! the web ([`pictures`]), which its content and its page name by id.

mod coach;
mod pictures;

pub(crate) use pictures::{DOWNLOAD_WITHIN, Pictures};

use crate::approvals::{self, Asker};
use async_trait::async_trait;
use niles_api::chat::Turn;
use niles_artifacts::{
    Artifact, ArtifactRef, ArtifactStore, Author, Filter, NewArtifact, NewRevision, PageChange,
    Status, StatusChange, kinds, page,
};
use niles_core::EventBus;
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Jobs one person may have going at once. A third waits for one of
/// them, so a model that loops cannot start a dozen.
pub(crate) const MAX_JOBS: usize = 2;

/// However long the run's own timeout is, a job is over by then.
const JOB_LIMIT: Duration = Duration::from_secs(30 * 60);

/// Why a job that was running is not any more, after a restart.
pub(crate) const RESTARTED: &str = "Niles restarted while this was being written.";

/// How long a turn's artifacts are remembered for the turn to pick up.
const TOUCHED_FOR: Duration = Duration::from_secs(60 * 60);

/// A final answer this short is a sentence about the document, not the
/// document: a job that saved nothing and said "Done." has failed.
const LEAST_LINES: usize = 3;

const TRAINING_PLAN: &str = "training_plan";
const TRIP: &str = "trip";

/// Told when a job ends, ready or failed, if the artifact's notify is
/// on.
#[async_trait]
pub(crate) trait Notifier: Send + Sync {
    async fn finished(&self, artifact: &Artifact);

    /// An artifact's page, made by a job the person asked for, is there.
    async fn page_ready(&self, _artifact: &Artifact) {}

    /// A training plan the coach changed after a run, with the note it
    /// left for the person. Called only when sessions changed, never
    /// for a look that left them as they were or only added a note, so
    /// a review after every run does not buzz the phone after every
    /// run; and only when the artifact's notify is on. Not `finished`,
    /// whose "ready" would be wrong for a plan that was there already.
    async fn changed(&self, _artifact: &Artifact, _note: &str) {}
}

/// Tells nobody: there is no push without a database and its secrets
/// key. The person still sees the status line, the Saved tab and the
/// card in the chat.
pub(crate) struct Nobody;

#[async_trait]
impl Notifier for Nobody {
    async fn finished(&self, _artifact: &Artifact) {}
}

/// Tells the person on every device where they turned notifications on.
/// Nobody has to have asked for it: a device that allowed notifications
/// is the asking.
pub(crate) struct Devices(pub(crate) Arc<niles_webpush::WebPush>);

impl Devices {
    async fn tell(&self, artifact: &Artifact, message: &niles_webpush::Message) {
        match self.0.send(&artifact.person, message).await {
            Ok(delivery) if delivery.sent > 0 => tracing::info!(
                "[artifacts] {}: told {} device(s) about {:?}",
                artifact.person,
                delivery.sent,
                artifact.title
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                "[artifacts] {}: nobody was told about {:?}: {e}",
                artifact.person,
                artifact.title
            ),
        }
    }
}

#[async_trait]
impl Notifier for Devices {
    async fn finished(&self, artifact: &Artifact) {
        self.tell(artifact, &finished_message(artifact)).await;
    }

    async fn changed(&self, artifact: &Artifact, note: &str) {
        self.tell(artifact, &changed_message(artifact, note)).await;
    }

    async fn page_ready(&self, artifact: &Artifact) {
        self.tell(artifact, &page_message(artifact)).await;
    }
}

/// The longest title a notification carries; a lock screen shows less.
const TITLE_IN_PUSH: usize = 120;

/// What a finished job's notification says, and where tapping it goes:
/// the artifact, open in the chat's Saved tab. One per artifact, so a
/// job tried again replaces the notification of the one that failed.
pub(crate) fn finished_message(artifact: &Artifact) -> niles_webpush::Message {
    let title: String = artifact.title.chars().take(TITLE_IN_PUSH).collect();
    let body = match artifact.status {
        Status::Failed => format!("Your {title} didn't finish. Open it to try again."),
        _ => format!("Your {title} is ready."),
    };
    niles_webpush::Message {
        title: "Niles".into(),
        body,
        url: format!("/#/chat?artifact={}", artifact.id),
        tag: format!("artifact-{}", artifact.id),
    }
}

/// What a finished page job's notification says: under the artifact's
/// tag, so it replaces what was said about the artifact before.
pub(crate) fn page_message(artifact: &Artifact) -> niles_webpush::Message {
    let title: String = artifact.title.chars().take(TITLE_IN_PUSH).collect();
    niles_webpush::Message {
        title: "Niles".into(),
        body: format!("The page for your {title} is ready."),
        url: format!("/#/chat?artifact={}", artifact.id),
        tag: format!("artifact-{}", artifact.id),
    }
}

/// The longest of the coach's note a notification carries: the rest is
/// in the plan, a tap away.
const NOTE_IN_PUSH: usize = 240;

/// What a plan the coach changed after a run says: that it changed, and
/// the coach's note. The same tag as its job's notification, so the
/// newest about a plan is the one on the lock screen.
pub(crate) fn changed_message(artifact: &Artifact, note: &str) -> niles_webpush::Message {
    let title: String = artifact.title.chars().take(TITLE_IN_PUSH).collect();
    let note = niles_artifacts::summary(note);
    let note: String = if note.chars().count() > NOTE_IN_PUSH {
        let cut: String = note.chars().take(NOTE_IN_PUSH - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        note
    };
    niles_webpush::Message {
        title: "Niles".into(),
        body: format!("Your {title} changed after your run. {note}"),
        url: format!("/#/chat?artifact={}", artifact.id),
        tag: format!("artifact-{}", artifact.id),
    }
}

/// Whatever runs a job: the assistant, in the background.
#[async_trait]
pub(crate) trait Runner: Send + Sync {
    /// `claude` or `chatgpt`, whichever would run a job now.
    fn available(&self) -> Option<&'static str>;

    /// Run the job with only `tools`, acting for `asker`. Answers which
    /// assistant ran it and its final answer.
    async fn run(
        &self,
        asker: Asker,
        system: &str,
        prompt: &str,
        tools: ToolRegistry,
    ) -> std::result::Result<(&'static str, String), String>;
}

/// The assistant chosen now, as the chat and the review choose it.
pub(crate) struct AssistantRunner {
    /// Set once the assistant has started: it serves the artifact tools,
    /// so it is started after them.
    pub(crate) assistant: std::sync::OnceLock<Arc<crate::assistant::Assistant>>,
    pub(crate) settings: Arc<niles_config::ConfigStore>,
}

impl AssistantRunner {
    fn chosen(&self) -> Option<crate::assistant::Chosen> {
        self.assistant.get()?;
        match crate::assistant::choose(&self.settings.current().integrations) {
            crate::assistant::Choice::Use(chosen) => Some(chosen),
            _ => None,
        }
    }
}

#[async_trait]
impl Runner for AssistantRunner {
    fn available(&self) -> Option<&'static str> {
        self.chosen().map(|c| c.via())
    }

    async fn run(
        &self,
        asker: Asker,
        system: &str,
        prompt: &str,
        tools: ToolRegistry,
    ) -> std::result::Result<(&'static str, String), String> {
        let (Some(assistant), Some(chosen)) = (self.assistant.get(), self.chosen()) else {
            return Err(NO_ASSISTANT.into());
        };
        let system = format!(
            "{system}\n\n{}",
            crate::prompt::home_context(&self.settings.current().home)
        );
        assistant
            .background(&chosen, &system, prompt, tools, Some(asker))
            .await
            .map(|answer| (chosen.via(), answer))
            .map_err(|e| format!("{} did not finish: {e:#}", chosen.name()))
    }
}

const NO_ASSISTANT: &str = "No assistant is on: Claude or ChatGPT writes these. Switch one on \
    under Settings → Voice & AI.";

/// Where a finished job's card goes: the conversation it came from.
#[async_trait]
pub(crate) trait Conversations: Send + Sync {
    async fn append(
        &self,
        person: &str,
        conversation: i64,
        turn: &Turn,
    ) -> std::result::Result<(), String>;
}

#[async_trait]
impl Conversations for crate::chat_log::ChatLog {
    async fn append(
        &self,
        person: &str,
        conversation: i64,
        turn: &Turn,
    ) -> std::result::Result<(), String> {
        self.record(person, Some(conversation), turn)
            .await
            .map(|_| ())
    }
}

/// What a job may use besides its own artifact: web search, the date.
pub(crate) type Extras = Box<dyn Fn(&mut ToolRegistry) + Send + Sync>;

/// An artifact a turn made or changed, until the turn is kept.
struct Touch {
    person: String,
    id: i64,
    at: Instant,
}

pub(crate) struct Artifacts {
    store: Arc<dyn ArtifactStore>,
    bus: EventBus,
    runner: Arc<dyn Runner>,
    notifier: Arc<dyn Notifier>,
    conversations: Option<Arc<dyn Conversations>>,
    extras: Extras,
    /// What a training plan's job reads besides: the person's own
    /// workouts.
    training: Option<Extras>,
    /// The house's, for the days a plan is kept by.
    tz: chrono_tz::Tz,
    /// Plans being looked at after a run: one look at a time each.
    reviewing: Arc<Mutex<HashSet<i64>>>,
    /// Artifacts a page is being made for: one at a time each.
    paging: Arc<Mutex<HashSet<i64>>>,
    /// Where pictures are found and fetched from; none, no pictures.
    pictures: Option<Pictures>,
    /// Jobs going, per person.
    running: Arc<Mutex<HashMap<String, usize>>>,
    touched: Mutex<Vec<Touch>>,
    /// When this run began: a job that last moved before it was the
    /// last run's.
    started: chrono::DateTime<chrono::Utc>,
}

/// One person's place among the jobs going, given back when the job
/// ends however it ends.
struct Slot {
    running: Arc<Mutex<HashMap<String, usize>>>,
    person: String,
}

/// An artifact's place among the ones being looked at, or given a page,
/// given back when that ends however it ends.
pub(crate) struct Held {
    ids: Arc<Mutex<HashSet<i64>>>,
    id: i64,
}

impl Held {
    pub(crate) fn claim(ids: &Arc<Mutex<HashSet<i64>>>, id: i64) -> Option<Self> {
        let mut held = ids.lock().unwrap_or_else(|e| e.into_inner());
        held.insert(id).then(|| Self {
            ids: ids.clone(),
            id,
        })
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = running.get_mut(&self.person) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                running.remove(&self.person);
            }
        }
    }
}

fn invalid(tool: &str, reason: impl Into<String>) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason: reason.into(),
    }
}

fn refused(tool: &str, e: niles_artifacts::Error) -> Error {
    match e {
        niles_artifacts::Error::Invalid { .. } => invalid(tool, e.to_string()),
        niles_artifacts::Error::Storage(_) => Error::Internal(e.to_string()),
    }
}

/// Whose artifacts this turn may touch: the asker's, when Niles knows
/// who that is.
fn asker(tool: &str) -> Result<Asker> {
    match approvals::asker() {
        Some(a) if !a.person.starts_with("satellite:") => Ok(a),
        _ => Err(invalid(
            tool,
            "I don't know who is asking, so there is nowhere to keep this. Saved artifacts \
             are each person's own: Niles has to know their voice, or they are signed in to \
             the app",
        )),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn id_arg(tool: &str, args: &Value) -> Result<i64> {
    args.get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
        .ok_or_else(|| invalid(tool, "id is the artifact's number, from artifact_list"))
}

/// What a tool answers about an artifact: enough to say what it is.
fn view(a: &Artifact) -> Value {
    json!({
        "id": a.id,
        "kind": a.kind,
        "title": a.title,
        "summary": a.summary,
        "status": a.status.as_str(),
        "status_text": a.status_text,
        "error": a.error,
        "version": a.current_revision,
        "page": a.page.is_some(),
        "changed": a.updated_at.to_rfc3339(),
        "preview": a.preview.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
    })
}

fn kinds_help() -> String {
    kinds::KINDS
        .iter()
        .map(|k| format!("{}: {}", k.id, k.content_help))
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_ids() -> Vec<&'static str> {
    kinds::KINDS.iter().map(|k| k.id).collect()
}

impl Artifacts {
    pub(crate) fn new(
        store: Arc<dyn ArtifactStore>,
        bus: EventBus,
        runner: Arc<dyn Runner>,
        notifier: Arc<dyn Notifier>,
        conversations: Option<Arc<dyn Conversations>>,
        extras: Extras,
    ) -> Self {
        Self {
            store,
            bus,
            runner,
            notifier,
            conversations,
            extras,
            training: None,
            tz: chrono_tz::UTC,
            reviewing: Arc::default(),
            paging: Arc::default(),
            pictures: None,
            running: Arc::default(),
            touched: Mutex::default(),
            started: chrono::Utc::now(),
        }
    }

    /// Lets a training plan's job read the asker's workouts, with the
    /// tools `register` adds, which act for whoever the job is for; and
    /// keeps plans by the days of `tz`, the house's.
    pub(crate) fn with_training(mut self, register: Extras, tz: chrono_tz::Tz) -> Self {
        self.training = Some(register);
        self.tz = tz;
        self
    }

    /// Lets the assistant keep pictures with an artifact: found with
    /// `pictures.search`, fetched with `pictures.download`.
    pub(crate) fn with_pictures(mut self, pictures: Pictures) -> Self {
        self.pictures = Some(pictures);
        self
    }

    pub(crate) fn store(&self) -> &Arc<dyn ArtifactStore> {
        &self.store
    }

    /// Which assistant would make one now.
    pub(crate) fn assistant(&self) -> Option<&'static str> {
        self.runner.available()
    }

    /// The tools the assistant is offered in a turn.
    pub(crate) fn tools(self: &Arc<Self>) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(List(self.clone())));
        reg.register(Box::new(Get(self.clone())));
        reg.register(Box::new(Create(self.clone())));
        reg.register(Box::new(Update {
            artifacts: self.clone(),
            only: None,
            keep_past: None,
        }));
        reg.register(Box::new(SetPage {
            artifacts: self.clone(),
            only: None,
        }));
        reg.register(Box::new(StartJob(self.clone())));
        reg.register(Box::new(StartPage(self.clone())));
        pictures::register(self, &mut reg, None);
        reg
    }

    /// What a job's run may use: its person's artifacts to read, its own
    /// to write, and what writing needs: for a training plan, their
    /// workouts; for a document or a trip, pictures. Not the house. A
    /// review of a plan saves only what leaves the days before
    /// `keep_past` as they were, and leaves its page alone: it has no
    /// way to set one.
    fn job_tools(
        self: &Arc<Self>,
        id: i64,
        kind: &str,
        keep_past: Option<chrono::NaiveDate>,
    ) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(List(self.clone())));
        reg.register(Box::new(Get(self.clone())));
        reg.register(Box::new(Update {
            artifacts: self.clone(),
            only: Some(id),
            keep_past,
        }));
        if keep_past.is_none() {
            reg.register(Box::new(SetPage {
                artifacts: self.clone(),
                only: Some(id),
            }));
        }
        (self.extras)(&mut reg);
        if let (TRAINING_PLAN, Some(training)) = (kind, &self.training) {
            training(&mut reg);
        }
        if keep_past.is_none() && kind != TRAINING_PLAN {
            pictures::register(self, &mut reg, Some(id));
        }
        reg
    }

    /// What a page's run may use: the person's artifacts to read, and
    /// this one's page and pictures to set. No web search: a page draws
    /// the data, and its pictures are found with `image_search`.
    fn page_tools(self: &Arc<Self>, id: i64) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(Get(self.clone())));
        reg.register(Box::new(SetPage {
            artifacts: self.clone(),
            only: Some(id),
        }));
        pictures::register(self, &mut reg, Some(id));
        reg
    }

    /// Mark every job a restart left behind as failed: its run was in
    /// the pod that went.
    pub(crate) async fn recover(&self) {
        match self.store.fail_abandoned(RESTARTED, self.started).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("[artifacts] {n} job(s) were cut short by a restart"),
            Err(e) => {
                tracing::warn!("[artifacts] could not look for jobs a restart cut short: {e}")
            }
        }
    }

    fn announce(&self, artifact: &Artifact) {
        niles_api::artifacts::announce(&self.bus, artifact);
    }

    fn touch(&self, person: &str, id: i64) {
        let now = Instant::now();
        let mut touched = self.touched.lock().unwrap_or_else(|e| e.into_inner());
        touched.retain(|t| now.duration_since(t.at) < TOUCHED_FOR);
        touched.retain(|t| !(t.person == person && t.id == id));
        touched.push(Touch {
            person: person.to_string(),
            id,
            at: now,
        });
    }

    fn touched_since(&self, person: &str, since: Instant) -> Vec<i64> {
        self.touched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|t| t.person == person && t.at >= since)
            .map(|t| t.id)
            .collect()
    }

    /// The cards for what a turn of `person`'s that began at `since`
    /// made or changed.
    pub(crate) async fn for_turn(&self, person: &str, since: Instant) -> Vec<ArtifactRef> {
        let mut out = Vec::new();
        for id in self.touched_since(person, since) {
            if let Ok(Some(a)) = self.store.get(person, id).await {
                out.push(ArtifactRef::of(&a));
            }
        }
        out
    }

    /// The conversation a turn was kept in, for the jobs it started: a
    /// new conversation has no id until its first turn is kept, which is
    /// after the job began.
    pub(crate) async fn placed(&self, person: &str, since: Instant, conversation: i64) {
        for id in self.touched_since(person, since) {
            let Ok(Some(a)) = self.store.get(person, id).await else {
                continue;
            };
            if a.conversation_id.is_none()
                && let Err(e) = self.store.set_conversation(person, id, conversation).await
            {
                tracing::warn!("[artifacts] where artifact {id} came from was not kept: {e}");
            }
        }
    }

    /// The cards in a conversation, as the artifacts are now: renamed,
    /// finished, or gone.
    pub(crate) async fn refresh(&self, person: &str, turns: &mut [Turn]) {
        for turn in turns.iter_mut().filter(|t| !t.artifacts.is_empty()) {
            let mut now = Vec::with_capacity(turn.artifacts.len());
            for card in &turn.artifacts {
                match self.store.get(person, card.id).await {
                    Ok(Some(a)) => now.push(ArtifactRef::of(&a)),
                    Ok(None) => {}
                    // Unreadable now is not gone: the card stays as it was.
                    Err(_) => now.push(card.clone()),
                }
            }
            turn.artifacts = now;
        }
    }

    fn take_slot(&self, person: &str) -> Option<Slot> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let n = running.entry(person.to_string()).or_default();
        if *n >= MAX_JOBS {
            return None;
        }
        *n += 1;
        Some(Slot {
            running: self.running.clone(),
            person: person.to_string(),
        })
    }

    /// Make the artifact, marked working, and start its job.
    async fn start(
        self: &Arc<Self>,
        asker: &Asker,
        job: Job,
    ) -> std::result::Result<Artifact, String> {
        if self.runner.available().is_none() {
            return Err(NO_ASSISTANT.into());
        }
        let slot = self.take_slot(&asker.person).ok_or_else(busy)?;
        let artifact = self
            .store
            .create(
                &asker.person,
                NewArtifact {
                    kind: job.kind.id.to_string(),
                    title: job.title,
                    summary: job.summary,
                    notify: job.notify,
                    status: Status::Working,
                    status_text: Some(job.status_text),
                    job_id: Some(new_job_id()),
                    brief: Some(job.brief),
                    conversation_id: asker.conversation,
                    first: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        self.announce(&artifact);
        self.spawn(artifact.clone(), slot);
        Ok(artifact)
    }

    /// A failed job, run again on the same artifact with its brief.
    pub(crate) async fn retry(
        self: &Arc<Self>,
        person: &str,
        id: i64,
    ) -> std::result::Result<Artifact, niles_api::artifacts::Refused> {
        use niles_api::artifacts::Refused;
        let storage = |e: niles_artifacts::Error| Refused::Storage(e.to_string());
        let artifact = self
            .store
            .get(person, id)
            .await
            .map_err(storage)?
            .ok_or(Refused::NotFound)?;
        if artifact.status != Status::Failed || artifact.brief.is_none() {
            return Err(Refused::NotRetryable(
                "only a job that did not finish can be tried again".into(),
            ));
        }
        if self.runner.available().is_none() {
            return Err(Refused::Unavailable(NO_ASSISTANT.into()));
        }
        let slot = self
            .take_slot(person)
            .ok_or_else(|| Refused::Unavailable(busy()))?;
        let status_text = artifact
            .status_text
            .clone()
            .unwrap_or_else(|| doing(&artifact.kind, &artifact.title));
        let artifact = self
            .store
            .set_status(
                person,
                id,
                StatusChange {
                    status: Status::Working,
                    status_text: Some(status_text),
                    error: None,
                    job_id: Some(new_job_id()),
                },
            )
            .await
            .map_err(storage)?
            .ok_or(Refused::NotFound)?;
        self.announce(&artifact);
        self.spawn(artifact.clone(), slot);
        Ok(artifact)
    }

    fn spawn(self: &Arc<Self>, artifact: Artifact, slot: Slot) {
        let this = self.clone();
        tokio::spawn(async move { this.run(artifact, slot).await });
    }

    async fn run(self: Arc<Self>, artifact: Artifact, _slot: Slot) {
        let person = artifact.person.clone();
        let asker = Asker {
            person: person.clone(),
            slug: person.strip_prefix("person:").map(str::to_string),
            satellite: None,
            conversation: artifact.conversation_id,
        };
        let (system, prompt) = job_prompts(&artifact);
        let tools = self.job_tools(artifact.id, &artifact.kind, None);
        let outcome =
            tokio::time::timeout(JOB_LIMIT, self.runner.run(asker, &system, &prompt, tools))
                .await
                .unwrap_or_else(|_| Err("it took longer than half an hour".into()));

        let now = match self.store.get(&person, artifact.id).await {
            Ok(Some(now)) => now,
            // Deleted while it was being written: nothing to finish.
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("[artifacts] job {} lost its artifact: {e}", artifact.id);
                return;
            }
        };
        // Started again since, by a retry that is now the one writing it.
        if now.job_id != artifact.job_id {
            return;
        }
        let ended = match outcome {
            Ok((via, answer)) => self.keep_answer(&now, via, &answer).await.map(|()| via),
            Err(why) => Err(why),
        };
        let change = match &ended {
            Ok(_) => StatusChange {
                status: Status::Ready,
                status_text: None,
                error: None,
                job_id: None,
            },
            Err(why) => StatusChange {
                status: Status::Failed,
                status_text: now.status_text.clone(),
                error: Some(why.clone()),
                job_id: None,
            },
        };
        let done = match self.store.set_status(&person, now.id, change).await {
            Ok(Some(done)) => done,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("[artifacts] job {} ended but was not kept: {e}", now.id);
                return;
            }
        };
        match &ended {
            Ok(_) => tracing::info!("[artifacts] {person}: {:?} is ready", done.title),
            Err(why) => tracing::warn!("[artifacts] {person}: {:?} failed: {why}", done.title),
        }
        self.announce(&done);
        if let Ok(via) = ended {
            self.say(
                &done,
                done.conversation_id,
                format!("{} is ready.", done.title),
                Some(via),
            )
            .await;
        }
        if done.notify {
            self.notifier.finished(&done).await;
        }
    }

    /// Start a run making a page for an artifact of `asker`'s that is
    /// there already. The artifact is not marked working: its data goes
    /// on being kept up while the page is made (a run marked on a plan,
    /// a tick), and the page is set over the data as it is when it is
    /// saved.
    async fn start_page(
        self: &Arc<Self>,
        asker: &Asker,
        id: i64,
        brief: String,
    ) -> std::result::Result<Artifact, String> {
        if self.runner.available().is_none() {
            return Err(NO_ASSISTANT.into());
        }
        let artifact = self
            .store
            .get(&asker.person, id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("there is no artifact {id}"))?;
        if artifact.status == Status::Working || artifact.current_revision == 0 {
            return Err(
                "it is still being written; a page can be made for it once it is there".into(),
            );
        }
        let held = Held::claim(&self.paging, id)
            .ok_or_else(|| "a page is being made for it already".to_string())?;
        let slot = self.take_slot(&asker.person).ok_or_else(busy)?;
        let this = self.clone();
        let (made, conversation) = (artifact.clone(), asker.conversation);
        tokio::spawn(async move { this.run_page(made, brief, conversation, slot, held).await });
        Ok(artifact)
    }

    async fn run_page(
        self: Arc<Self>,
        artifact: Artifact,
        brief: String,
        conversation: Option<i64>,
        _slot: Slot,
        _held: Held,
    ) {
        let person = artifact.person.clone();
        let asker = Asker {
            person: person.clone(),
            slug: person.strip_prefix("person:").map(str::to_string),
            satellite: None,
            conversation,
        };
        let (system, prompt) = page_prompts(&artifact, &brief);
        let tools = self.page_tools(artifact.id);
        let outcome =
            tokio::time::timeout(JOB_LIMIT, self.runner.run(asker, &system, &prompt, tools))
                .await
                .unwrap_or_else(|_| Err("it took longer than half an hour".into()));
        let ended = match outcome {
            Ok((via, answer)) => self.keep_page(&artifact, via, &answer).await.map(|()| via),
            Err(why) => Err(why),
        };
        let now = match self.store.get(&person, artifact.id).await {
            Ok(Some(now)) => now,
            // Deleted while the page was being made.
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("[artifacts] page {} lost its artifact: {e}", artifact.id);
                return;
            }
        };
        let conversation = conversation.or(now.conversation_id);
        match ended {
            Ok(via) => {
                tracing::info!("[artifacts] {person}: {:?} has a new page", now.title);
                self.announce(&now);
                let reply = format!("The page for {} is ready.", now.title);
                self.say(&now, conversation, reply, Some(via)).await;
                if now.notify {
                    self.notifier.page_ready(&now).await;
                }
            }
            Err(why) => {
                tracing::warn!("[artifacts] {person}: no page for {:?}: {why}", now.title);
                let reply = format!("The page for {} didn't get made: {why}", now.title);
                self.say(&now, conversation, reply, None).await;
            }
        }
    }

    /// A page run that saved no page may have answered with it; that is
    /// kept as the page.
    async fn keep_page(
        &self,
        before: &Artifact,
        via: &'static str,
        answer: &str,
    ) -> std::result::Result<(), String> {
        let now = self
            .store
            .get(&before.person, before.id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("it was deleted")?;
        if now.page != before.page {
            return Ok(());
        }
        let html = page::from_answer(answer).ok_or("it finished without saving a page")?;
        let html = page::page(&html).map_err(|e| e.to_string())?;
        self.store
            .set_page(
                &before.person,
                before.id,
                PageChange::New(html),
                "A page for it".into(),
                author(Some(via)),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A run that saved nothing may still have answered with the
    /// document itself; that is kept as the first version.
    async fn keep_answer(
        &self,
        artifact: &Artifact,
        via: &'static str,
        answer: &str,
    ) -> std::result::Result<(), String> {
        if artifact.current_revision > 0 {
            return Ok(());
        }
        let lines = answer.lines().filter(|l| !l.trim().is_empty()).count();
        if lines < LEAST_LINES {
            return Err("it finished without saving anything".into());
        }
        let content = kinds::validate(&artifact.kind, &Value::String(answer.to_string()))
            .map_err(|e| e.to_string())?;
        self.store
            .revise(
                &artifact.person,
                artifact.id,
                NewRevision {
                    content,
                    reason: "First draft".into(),
                    author: author(Some(via)),
                    summary: None,
                    page: niles_artifacts::PageChange::Keep,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A card in the conversation a job came from, with what to say
    /// about it: that it is ready, or why it is not.
    async fn say(
        &self,
        artifact: &Artifact,
        conversation: Option<i64>,
        reply: String,
        by: Option<&'static str>,
    ) {
        let (Some(conversations), Some(conversation)) = (&self.conversations, conversation) else {
            return;
        };
        let turn = Turn {
            at: chrono::Utc::now(),
            said: String::new(),
            reply,
            via: "app".into(),
            room: None,
            by: by.map(str::to_string),
            proposal: None,
            approval: None,
            artifacts: vec![ArtifactRef::of(artifact)],
            attachments: Vec::new(),
        };
        if let Err(e) = conversations
            .append(&artifact.person, conversation, &turn)
            .await
        {
            tracing::warn!("[artifacts] the card for {} was not kept: {e}", artifact.id);
        }
    }
}

/// What a job is asked to make.
struct Job {
    kind: &'static kinds::Kind,
    title: String,
    summary: String,
    brief: String,
    status_text: String,
    notify: bool,
}

fn busy() -> String {
    format!(
        "{MAX_JOBS} are being written for this person already; another can start when one \
         of them is done"
    )
}

fn new_job_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// "Writing “Packing list”…", when the assistant gave nothing better.
fn doing(kind: &str, title: &str) -> String {
    let verb = kinds::kind(kind).map_or("Writing", |k| k.doing);
    format!("{verb} “{title}”…")
}

fn author(via: Option<&str>) -> Author {
    match via {
        Some("claude") => Author::Claude,
        Some("chatgpt") => Author::Chatgpt,
        _ => Author::Niles,
    }
}

/// How a document is written well.
const RESEARCH: &str = "Where facts matter (opening times, ferries, prices, the weather in a \
    season), check them with web_search and read_page when you have them, and say in the \
    document what you could not check.";

/// How a training plan is made well: from the runs the person has done,
/// not from a template.
const COACHING: &str = "Build it from how they run now. Find today's date first. Read \
    training_summary for the last quarter and the last month, and training_activities for \
    their runs of the last six weeks: how far they run in a week, their longest run lately, \
    and the paces of their easy and their faster runs. The first week is close to what they \
    already do; from there the weekly distance grows by no more than about 10% a week, with \
    an easier week every third or fourth, the long run growing toward the race (to 16-20 km \
    for a half marathon), a taper in the last one or two weeks, and the race on its day as \
    type race. Easy runs make most of the distance, with one quality session a week (tempo \
    or intervals), two at most. Paces come from their recent runs and the target time, and \
    are honest about any gap between the two. If Strava has nothing for them, start gently \
    and say so. Where the brief does not say (the race date, a target, which days they can \
    run, how many runs a week), assume what is usual (three or four runs, the long run at \
    the weekend, a rest day after a hard one) and say in notes what you assumed, so they can \
    ask for it changed.";

/// How a trip is planned well: one they could set out on, not a
/// brochure. Researched as a document is, after this.
const TRAVEL: &str = "Find today's date first, and plan the trip in the brief, not an ideal \
    one: the dates, places and pace they asked for. Days are for the travellers, not the \
    plan: one or two things that matter in a day, with time to eat, rest and get lost, and \
    a day of travel is mostly that. Get the travel between places right (the flight, train, \
    bus or ferry there is, how long it really takes door to door, and from where), and put \
    it on the day as a travel item. Mark nothing as booked unless the brief says it is, \
    with its confirmation if it gives one. Prefer official links (the attraction's, the \
    operator's, the hotel's own) to a booking site's or a blog's. Prices are estimates in \
    the local currency, said as estimates. Make a packing list that fits where they go and \
    the weather there in that season, grouped (documents, clothes, toiletries, health, \
    electronics). Where the brief does not say (where they fly from, how they like to \
    travel, the budget), assume what is usual and say in notes what you assumed and what \
    you could not check, so they can ask for it changed.";

/// Pictures where they help the person, kept by Niles.
const PICTURES: &str = "Where a picture helps (the places of a trip, a guide to somewhere, \
    a recipe), add a few, and only ones that show what the text is about; none is better \
    than a wrong one.";

/// A job's look is its kind's own view, unless the brief asks for
/// another.
const LOOK: &str = "The app shows it with its kind's own view. Only if the brief asks for it \
    to look a particular way, give it a page as well with artifact_set_page, once the content \
    is saved.";

/// How a page is made well: for this artifact and its person, drawing
/// what the data has.
const DESIGN: &str = "Make it look made for this, and for a phone: a clear order from the \
    most important down, generous spacing, the app's colours and radius, readable at a \
    glance. Draw what the data has rather than repeating it as text: a countdown, a week's \
    kilometres as bars, the route of a trip day by day. Keep everything the kind's own view \
    shows that matters to the person (for a training plan each session and whether it is \
    done, for a trip each day, its bookings and the packing list), and keep what the person \
    ticks there tickable, saved with niles.save. Do what the brief asks above all of this.";

/// A page run's instructions, and what it is to make.
fn page_prompts(artifact: &Artifact, brief: &str) -> (String, String) {
    let kind = kinds::kind(&artifact.kind);
    let label = kind.map_or("document", |k| k.label).to_lowercase();
    let help = kind.map_or("", |k| k.content_help);
    let system = format!(
        "You are making a page for one person's {label} in the household Niles serves, in \
         the background. Nobody is waiting on you in a chat: take the time to do it well, \
         and ask nothing, because nobody will answer. Read it first with artifact_get on \
         artifact {id}, with with_page: true. Then write the page and save it with \
         artifact_set_page on artifact {id}, with a short reason; if it is refused, the \
         answer says what to fix, so fix that and save again. Then finish with one short \
         sentence saying what you made.\n\nThe data it draws is {help}\n\n{}\n\n{DESIGN} \
         The pictures it keeps are listed by artifact_get; if the brief asks for pictures it \
         does not have, {}\n\nWrite any words on the page in the language of the {label}.",
        page::HELP,
        niles_artifacts::images::HELP,
        id = artifact.id,
    );
    let prompt = format!(
        "Title: {title}\nSummary: {summary}\n\nWhat they asked the page to be, from the \
         conversation it was asked for in:\n\n{brief}",
        title = artifact.title,
        summary = artifact.summary,
    );
    (system, prompt)
}

/// The run's instructions, and what it is to make.
fn job_prompts(artifact: &Artifact) -> (String, String) {
    let kind = kinds::kind(&artifact.kind);
    let label = kind.map_or("document", |k| k.label).to_lowercase();
    let help = kind.map_or("", |k| k.content_help);
    let craft = match artifact.kind.as_str() {
        TRAINING_PLAN => COACHING.to_string(),
        TRIP => format!("{TRAVEL} {RESEARCH} {PICTURES}"),
        _ => format!("{RESEARCH} {PICTURES}"),
    };
    let craft = format!("{craft} {LOOK}");
    let system = format!(
        "You are writing a {label} for one person in the household Niles serves, in the \
         background. Nobody is waiting on you in a chat: take the time to do it well, and \
         ask nothing, because nobody will answer. When it is written, save it with \
         artifact_update on artifact {id}, the whole content, with the reason \"First \
         draft\" and a one-line summary; if it is refused, the answer says what to fix, so \
         fix that and save again. Then finish with one short sentence saying what you \
         wrote.\n\nThe content is {help}\n\n{craft} Write in the language the brief is in.",
        id = artifact.id,
    );
    let prompt = format!(
        "Title: {title}\nSummary so far: {summary}\n\nThe brief, from the conversation it \
         was asked for in:\n\n{brief}",
        title = artifact.title,
        summary = artifact.summary,
        brief = artifact.brief.as_deref().unwrap_or("(none)"),
    );
    (system, prompt)
}

/// What the assistant is told about artifacts in a turn, beside the
/// tools themselves.
pub(crate) const PROMPT: &str = "\n\n# Saved for the person\n\nWhat the person will want to \
    keep, such as a list, a plan or notes, you can save for them: artifact_create, which they \
    find under Saved in the app, with a card in the chat. Change one with artifact_update. \
    For anything big (research, several sections, more than a minute of writing), call \
    start_artifact_job and answer in one sentence that you are putting it together and they \
    will find it under Saved. Do not write it out in the chat as well.\n\nA training plan \
    toward a race is kind training_plan, never a document: start_artifact_job, whose run \
    reads their Strava itself. Ask first for the race and its date if you do not know them; \
    the rest (a target time, which days they can run) the run can assume, and put in the \
    brief what they said about it. Once it is made it keeps up by itself: Niles marks each \
    run from Strava on it as it comes in, and after a run you look it over in the background \
    and adjust what is to come if that is needed.\n\nA travel plan or itinerary is kind trip, \
    never a document: start_artifact_job, whose run researches it. Ask first for the dates and \
    roughly where they want to go if you do not know them; put in the brief who is going, \
    what they have booked already, and what they said about pace, budget and what they \
    like.\n\nAn artifact can have a page as well: HTML you write once that draws its data, \
    shown in place of its kind's own view. Only for when the look matters: they ask for it \
    to look nicer or different, or want something to show. The kind's own view is good \
    otherwise, so make no page unasked. For a page, call start_page_job with the \
    artifact's id and a brief of what they want, and answer in one sentence that it is \
    being made; a small change to a page there is (a colour, a heading) you can make \
    yourself with artifact_set_page. The page draws the data, so a change to the data \
    needs no new page.\n\nA picture they want on a document, a trip or a page: \
    image_search finds it, artifact_add_image keeps Niles's own copy and answers its name, \
    niles-image:ID, which the content or the page then uses. Never a picture's web \
    address: it is not shown.";

struct List(Arc<Artifacts>);

#[async_trait]
impl Tool for List {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_list".into(),
            description: "The saved artifacts of the person asking (documents, lists, plans), \
                newest first: id, kind, title, summary, status, and when each changed. \
                Archived ones only with archived: true."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "archived": { "type": "boolean", "description": "List the archived ones instead" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_list";
        let asker = asker(TOOL)?;
        let archived = args
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let found = self
            .0
            .store
            .list(
                &asker.person,
                Filter {
                    archived: Some(archived),
                    ..Filter::default()
                },
            )
            .await
            .map_err(|e| refused(TOOL, e))?;
        Ok(json!({ "artifacts": found.iter().map(view).collect::<Vec<_>>() }))
    }
}

struct Get(Arc<Artifacts>);

#[async_trait]
impl Tool for Get {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_get".into(),
            description: "One of the asking person's saved artifacts, with its whole current \
                content. Read it before changing it: they may have ticked items since. page \
                says whether it has a page; with_page: true gives the page's HTML as well."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "with_page": { "type": "boolean", "description": "The page's HTML too, as page_html" }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_get";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        let store = &self.0.store;
        let a = store
            .get(&asker.person, id)
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        let content = match a.current_revision {
            0 => None,
            n => store
                .revision(&asker.person, id, n)
                .await
                .map_err(|e| refused(TOOL, e))?
                .map(|r| r.content),
        };
        let mut out = view(&a);
        out["content"] = content.unwrap_or(Value::Null);
        let kept = store
            .images(&asker.person, id)
            .await
            .map_err(|e| refused(TOOL, e))?;
        out["images"] = json!(kept.iter().map(pictures::view).collect::<Vec<_>>());
        if let (Some(true), Some(n)) = (args.get("with_page").and_then(Value::as_bool), a.page) {
            out["page_html"] = store
                .page(&asker.person, id, n)
                .await
                .map_err(|e| refused(TOOL, e))?
                .map_or(Value::Null, Value::String);
        }
        Ok(out)
    }
}

struct Create(Arc<Artifacts>);

#[async_trait]
impl Tool for Create {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_create".into(),
            description: format!(
                "Save something for the person asking, to keep: it is theirs alone, under \
                 Saved in the app, with a card under your reply. For a list, a plan, notes, \
                 a recipe: whatever they will come back to, that you can write now. For \
                 anything big, use start_artifact_job instead. summary is one line saying \
                 what it is. Kinds: {}",
                kinds_help()
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": kind_ids() },
                    "title": { "type": "string", "description": "Short, as a person would name it: \"Packing list for the cabin\"" },
                    "summary": { "type": "string" },
                    "content": { "type": "object", "description": "The kind's content" },
                    "notify": { "type": "boolean", "description": "Tell the person when it changes; on unless they said not to" }
                },
                "required": ["kind", "title", "summary", "content"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_create";
        let asker = asker(TOOL)?;
        let kind = kinds::require(str_arg(&args, "kind").unwrap_or("document"))
            .map_err(|e| refused(TOOL, e))?;
        let title = niles_artifacts::title(str_arg(&args, "title").unwrap_or_default())
            .map_err(|e| refused(TOOL, e))?;
        let content = kind
            .validate(args.get("content").unwrap_or(&Value::Null))
            .map_err(|e| refused(TOOL, e))?;
        let note = pictures::missing(&content, &[]);
        let a = self
            .0
            .store
            .create(
                &asker.person,
                NewArtifact {
                    kind: kind.id.to_string(),
                    title,
                    summary: niles_artifacts::summary(
                        str_arg(&args, "summary").unwrap_or_default(),
                    ),
                    notify: args.get("notify").and_then(Value::as_bool).unwrap_or(true),
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: None,
                    conversation_id: asker.conversation,
                    first: Some(NewRevision {
                        content,
                        reason: "First version".into(),
                        author: author(crate::assistant::via()),
                        summary: None,
                        page: niles_artifacts::PageChange::Keep,
                    }),
                },
            )
            .await
            .map_err(|e| refused(TOOL, e))?;
        self.0.touch(&asker.person, a.id);
        self.0.announce(&a);
        let mut out = view(&a);
        if let Some(note) = note {
            out["note"] = json!(note);
        }
        Ok(out)
    }
}

struct Update {
    artifacts: Arc<Artifacts>,
    /// In a job, the one artifact it may write.
    only: Option<i64>,
    /// In a review of a training plan, today: what happened before it,
    /// and every session no longer planned, stays as it is, whatever
    /// the model was told ([`niles_artifacts::tracking::keeps_the_past`]).
    keep_past: Option<chrono::NaiveDate>,
}

#[async_trait]
impl Tool for Update {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_update".into(),
            description: "Save a new version of one of the asking person's artifacts: the \
                whole new content (not a change to it), and a short reason for the list of \
                versions (\"Added the ferry to Koh Tao\"). Read it with artifact_get first, \
                and keep what they ticked. summary, if it should change, is one line."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "content": { "type": "object", "description": "The whole content, as its kind has it" },
                    "summary": { "type": "string" },
                    "reason": { "type": "string" }
                },
                "required": ["id", "content", "reason"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_update";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        if self.only.is_some_and(|only| only != id) {
            return Err(invalid(
                TOOL,
                format!(
                    "this job writes artifact {}, and only that one",
                    self.only.unwrap_or(id)
                ),
            ));
        }
        let store = &self.artifacts.store;
        let a = store
            .get(&asker.person, id)
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        let content = kinds::validate(&a.kind, args.get("content").unwrap_or(&Value::Null))
            .map_err(|e| refused(TOOL, e))?;
        if let Some(today) = self.keep_past {
            let current = store
                .revision(&asker.person, id, a.current_revision)
                .await
                .map_err(|e| refused(TOOL, e))?
                .ok_or_else(|| invalid(TOOL, format!("artifact {id} has nothing to change yet")))?;
            niles_artifacts::tracking::keeps_the_past(&current.content, &content, today)
                .map_err(|e| refused(TOOL, e))?;
        }
        let reason = niles_artifacts::summary(str_arg(&args, "reason").unwrap_or("Changed"));
        let kept = store
            .images(&asker.person, id)
            .await
            .map_err(|e| refused(TOOL, e))?;
        let note = pictures::missing(&content, &kept);
        let a = store
            .revise(
                &asker.person,
                id,
                NewRevision {
                    content,
                    reason,
                    author: author(crate::assistant::via()),
                    summary: str_arg(&args, "summary").map(niles_artifacts::summary),
                    page: niles_artifacts::PageChange::Keep,
                },
            )
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        // A job's saves are its own; its end is what the person hears.
        if self.only.is_none() {
            self.artifacts.touch(&asker.person, a.id);
            self.artifacts.announce(&a);
        }
        let mut out = view(&a);
        if let Some(note) = note {
            out["note"] = json!(note);
        }
        Ok(out)
    }
}

struct SetPage {
    artifacts: Arc<Artifacts>,
    /// In a job, the one artifact it may give a page.
    only: Option<i64>,
}

#[async_trait]
impl Tool for SetPage {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_set_page".into(),
            description: format!(
                "Give one of the asking person's artifacts a page, or a new page, or take it \
                 away (remove: true, back to the kind's own view). The data is kept as it \
                 is; the page is a new version, so it can be undone. Read the artifact with \
                 artifact_get first, with with_page: true to change the page it has. reason \
                 is for the list of versions (\"A page with the route day by day\"). {}",
                page::HELP
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "html": { "type": "string", "description": "The whole page" },
                    "remove": { "type": "boolean", "description": "Take the page away instead" },
                    "reason": { "type": "string" }
                },
                "required": ["id", "reason"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_set_page";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        if self.only.is_some_and(|only| only != id) {
            return Err(invalid(
                TOOL,
                format!(
                    "this job makes the page of artifact {}, and only that one",
                    self.only.unwrap_or(id)
                ),
            ));
        }
        let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
        let (change, fallback) = if remove {
            (PageChange::Remove, "Took the page away")
        } else {
            let html = args
                .get("html")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(TOOL, "html is the whole page; or remove: true"))?;
            let html = page::page(html).map_err(|e| refused(TOOL, e))?;
            (PageChange::New(html), "A new page")
        };
        let reason = niles_artifacts::summary(str_arg(&args, "reason").unwrap_or(fallback));
        let a = self
            .artifacts
            .store
            .set_page(
                &asker.person,
                id,
                change,
                reason,
                author(crate::assistant::via()),
            )
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        if self.only.is_none() {
            self.artifacts.touch(&asker.person, a.id);
            self.artifacts.announce(&a);
        }
        Ok(view(&a))
    }
}

struct StartPage(Arc<Artifacts>);

#[async_trait]
impl Tool for StartPage {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "start_page_job".into(),
            description: "Make a page for one of the asking person's artifacts, in the \
                background: when they ask for it to look nicer or different, or for something \
                to show. A page is HTML that draws the artifact's data, shown in place of its \
                kind's own view; the data stays as it is. It returns at once; another run of \
                you makes the page, and the person is told when it is there. brief is what \
                they want it to look like, in their own words, because that run will not see \
                this conversation. Then answer in one sentence that the page is being made."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "brief": { "type": "string" }
                },
                "required": ["id", "brief"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "start_page_job";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        let brief = str_arg(&args, "brief")
            .unwrap_or("Make it look good: clear on a phone, in the app's colours.")
            .to_string();
        let a = self
            .0
            .start_page(&asker, id, brief)
            .await
            .map_err(|why| invalid(TOOL, why))?;
        self.0.touch(&asker.person, a.id);
        let mut out = view(&a);
        out["note"] = json!(
            "The page is being made now. Tell the person in one sentence; they will be told \
             when it is there."
        );
        Ok(out)
    }
}

struct StartJob(Arc<Artifacts>);

#[async_trait]
impl Tool for StartJob {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "start_artifact_job".into(),
            description: format!(
                "Make a big artifact for the person asking, in the background: a travel plan, \
                 a packing list for a long trip, anything that needs research, several \
                 sections, or more than a minute of writing. It returns at once; another run \
                 of you writes it, and the person sees it being written under Saved and is \
                 told when it is ready. brief is everything that run needs, because it will \
                 not see this conversation: dates, places, people, preferences, and what they \
                 asked for in their own words. status_text is what they see meanwhile, such \
                 as \"Writing your Thailand travel plan\". Then answer in one sentence that \
                 you are putting it together and they will find it under Saved. {MAX_JOBS} \
                 can be written at a time for one person. Kinds: {}",
                kind_ids().join(", ")
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": kind_ids() },
                    "title": { "type": "string" },
                    "summary": { "type": "string", "description": "One line saying what it will be" },
                    "brief": { "type": "string" },
                    "status_text": { "type": "string" },
                    "notify": { "type": "boolean", "description": "Tell the person when it is ready; on unless they said not to" }
                },
                "required": ["kind", "title", "summary", "brief"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "start_artifact_job";
        let asker = asker(TOOL)?;
        let kind = kinds::require(str_arg(&args, "kind").unwrap_or("document"))
            .map_err(|e| refused(TOOL, e))?;
        let title = niles_artifacts::title(str_arg(&args, "title").unwrap_or_default())
            .map_err(|e| refused(TOOL, e))?;
        let brief = str_arg(&args, "brief")
            .ok_or_else(|| invalid(TOOL, "brief is what the run is to make, and it is empty"))?
            .to_string();
        let status_text = match str_arg(&args, "status_text") {
            Some(text) => format!(
                "{}…",
                niles_artifacts::summary(text).trim_end_matches(['…', '.'])
            ),
            None => doing(kind.id, &title),
        };
        let job = Job {
            kind,
            title,
            summary: niles_artifacts::summary(str_arg(&args, "summary").unwrap_or_default()),
            brief,
            status_text,
            notify: args.get("notify").and_then(Value::as_bool).unwrap_or(true),
        };
        let a = self
            .0
            .start(&asker, job)
            .await
            .map_err(|why| invalid(TOOL, why))?;
        self.0.touch(&asker.person, a.id);
        let mut out = view(&a);
        out["note"] = json!(
            "It is being written now. Tell the person in one sentence that you are putting it \
             together and they will find it under Saved."
        );
        Ok(out)
    }
}

/// The API's way to the jobs.
pub(crate) struct AppJobs(pub(crate) Arc<Artifacts>);

#[async_trait]
impl niles_api::artifacts::Jobs for AppJobs {
    fn assistant(&self) -> Option<&'static str> {
        self.0.assistant()
    }

    async fn retry(
        &self,
        person: &str,
        id: i64,
    ) -> std::result::Result<Artifact, niles_api::artifacts::Refused> {
        self.0.retry(person, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_artifacts::MemoryArtifacts;
    use niles_core::Event;
    use std::collections::VecDeque;
    use tokio::sync::Notify;

    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";

    fn asking(person: &str, conversation: Option<i64>) -> Asker {
        Asker {
            person: person.into(),
            slug: person.strip_prefix("person:").map(str::to_string),
            satellite: None,
            conversation,
        }
    }

    /// What the fake assistant does with a job.
    enum Does {
        /// Saves this markdown through the job's own tool, as a real run
        /// would.
        Saves(&'static str),
        /// Saves nothing, and answers this.
        Answers(&'static str),
        Fails(&'static str),
        /// Waits to be let go, then saves.
        Waits(Arc<Notify>),
        /// Gives its artifact this page through the job's own tool.
        Pages(&'static str),
    }

    struct FakeRunner {
        on: bool,
        jobs: Mutex<VecDeque<Does>>,
    }

    impl FakeRunner {
        fn doing(jobs: Vec<Does>) -> Arc<Self> {
            Arc::new(Self {
                on: true,
                jobs: Mutex::new(jobs.into()),
            })
        }
    }

    /// The artifact a job's instructions name, as a run reads it.
    fn job_artifact(system: &str) -> i64 {
        let after = system
            .split("on artifact ")
            .nth(1)
            .expect("an artifact named");
        after
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    }

    async fn save(
        tools: &ToolRegistry,
        asker: Asker,
        working: i64,
        markdown: &str,
    ) -> std::result::Result<(), String> {
        approvals::ASKER
            .scope(
                asker,
                tools.execute(&niles_llm::ToolCall {
                    id: String::new(),
                    name: "artifact_update".into(),
                    arguments: json!({
                        "id": working,
                        "content": { "markdown": markdown },
                        "reason": "First draft",
                    }),
                }),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[async_trait]
    impl Runner for FakeRunner {
        fn available(&self) -> Option<&'static str> {
            self.on.then_some("claude")
        }

        async fn run(
            &self,
            asker: Asker,
            system: &str,
            _prompt: &str,
            tools: ToolRegistry,
        ) -> std::result::Result<(&'static str, String), String> {
            let does = self.jobs.lock().unwrap().pop_front().expect("a job to do");
            let id = job_artifact(system);
            match does {
                Does::Saves(markdown) => {
                    save(&tools, asker, id, markdown).await?;
                    Ok(("claude", "Wrote it.".into()))
                }
                Does::Answers(text) => Ok(("claude", text.into())),
                Does::Fails(why) => Err(why.into()),
                Does::Waits(go) => {
                    go.notified().await;
                    save(&tools, asker, id, "# done").await?;
                    Ok(("claude", "Wrote it.".into()))
                }
                Does::Pages(html) => {
                    approvals::ASKER
                        .scope(
                            asker,
                            tools.execute(&niles_llm::ToolCall {
                                id: String::new(),
                                name: "artifact_set_page".into(),
                                arguments: json!({ "id": id, "html": html, "reason": "A page" }),
                            }),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(("claude", "Made it.".into()))
                }
            }
        }
    }

    #[derive(Default)]
    struct Told(Mutex<Vec<(i64, Status)>>, Mutex<Vec<i64>>);

    #[async_trait]
    impl Notifier for Told {
        async fn finished(&self, artifact: &Artifact) {
            self.0.lock().unwrap().push((artifact.id, artifact.status));
        }

        async fn page_ready(&self, artifact: &Artifact) {
            self.1.lock().unwrap().push(artifact.id);
        }
    }

    #[derive(Default)]
    struct Chats(Mutex<Vec<(String, i64, Turn)>>);

    #[async_trait]
    impl Conversations for Chats {
        async fn append(
            &self,
            person: &str,
            conversation: i64,
            turn: &Turn,
        ) -> std::result::Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push((person.into(), conversation, turn.clone()));
            Ok(())
        }
    }

    struct Fixture {
        artifacts: Arc<Artifacts>,
        store: Arc<MemoryArtifacts>,
        told: Arc<Told>,
        chats: Arc<Chats>,
        bus: EventBus,
    }

    fn fixture(runner: Arc<dyn Runner>) -> Fixture {
        let store = Arc::new(MemoryArtifacts::new());
        let told = Arc::new(Told::default());
        let chats = Arc::new(Chats::default());
        let bus = EventBus::default();
        let artifacts = Arc::new(Artifacts::new(
            store.clone(),
            bus.clone(),
            runner,
            told.clone(),
            Some(chats.clone() as Arc<dyn Conversations>),
            Box::new(|_| {}),
        ));
        Fixture {
            artifacts,
            store,
            told,
            chats,
            bus,
        }
    }

    async fn call(f: &Fixture, asker: Asker, name: &str, args: Value) -> Result<Value> {
        let tools = f.artifacts.tools();
        approvals::ASKER
            .scope(
                asker,
                tools.execute(&niles_llm::ToolCall {
                    id: String::new(),
                    name: name.into(),
                    arguments: args,
                }),
            )
            .await
    }

    async fn create(f: &Fixture, person: &str, title: &str) -> i64 {
        call(
            f,
            asking(person, Some(1)),
            "artifact_create",
            json!({
                "kind": "document",
                "title": title,
                "summary": "What to bring",
                "content": { "markdown": "- [ ] passport\n- [ ] charger" },
            }),
        )
        .await
        .unwrap()["id"]
            .as_i64()
            .unwrap()
    }

    /// Until the artifact is no longer working, or two seconds pass.
    async fn settled(f: &Fixture, person: &str, id: i64) -> Artifact {
        for _ in 0..200 {
            let a = f.store.get(person, id).await.unwrap().unwrap();
            if a.status != Status::Working {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("artifact {id} was still being written");
    }

    async fn start(f: &Fixture, person: &str, conversation: Option<i64>) -> Result<Value> {
        call(
            f,
            asking(person, conversation),
            "start_artifact_job",
            json!({
                "kind": "document",
                "title": "Thailand travel plan",
                "summary": "Two weeks in March",
                "brief": "Two weeks in Thailand in March, Bangkok then the islands.",
                "status_text": "Writing your Thailand travel plan",
            }),
        )
        .await
    }

    #[tokio::test]
    async fn a_tool_acts_only_for_the_person_asking() {
        let f = fixture(FakeRunner::doing(vec![]));
        let his = create(&f, MARK, "Packing list").await;
        let hers = asking(MAJSE, None);
        let listed = call(&f, hers.clone(), "artifact_list", json!({}))
            .await
            .unwrap();
        assert!(listed["artifacts"].as_array().unwrap().is_empty());
        assert!(
            call(&f, hers.clone(), "artifact_get", json!({ "id": his }))
                .await
                .is_err()
        );
        let changed = call(
            &f,
            hers,
            "artifact_update",
            json!({ "id": his, "content": { "markdown": "mine" }, "reason": "x" }),
        )
        .await;
        assert!(changed.is_err());
        let still = f.store.get(MARK, his).await.unwrap().unwrap();
        assert_eq!(still.current_revision, 1);
    }

    #[tokio::test]
    async fn an_unrecognised_voice_has_no_artifacts() {
        let f = fixture(FakeRunner::doing(vec![]));
        let stranger = Asker {
            person: "satellite:10.0.0.9:1234".into(),
            slug: None,
            satellite: Some("10.0.0.9:1234".parse().unwrap()),
            conversation: None,
        };
        let e = call(&f, stranger, "artifact_list", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("who is asking"), "{e}");
        let outside = f.artifacts.tools();
        let e = outside
            .execute(&niles_llm::ToolCall {
                id: String::new(),
                name: "artifact_list".into(),
                arguments: json!({}),
            })
            .await;
        assert!(e.is_err(), "no asker at all is nobody");
    }

    #[tokio::test]
    async fn content_is_checked_for_its_kind_before_anything_is_kept() {
        let f = fixture(FakeRunner::doing(vec![]));
        let e = call(
            &f,
            asking(MARK, None),
            "artifact_create",
            json!({ "kind": "document", "title": "Empty", "summary": "", "content": { "markdown": " " } }),
        )
        .await;
        assert!(e.is_err());
        let e = call(
            &f,
            asking(MARK, None),
            "artifact_create",
            json!({ "kind": "spreadsheet", "title": "Sums", "summary": "", "content": {} }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("document"), "{e}");
        assert!(
            f.store
                .list(MARK, Filter::default())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_update_is_a_new_version_by_the_assistant() {
        let f = fixture(FakeRunner::doing(vec![]));
        let id = create(&f, MARK, "Packing list").await;
        let out = crate::assistant::tests_support::as_assistant(
            "chatgpt",
            call(
                &f,
                asking(MARK, None),
                "artifact_update",
                json!({ "id": id, "content": { "markdown": "- [ ] passport" }, "reason": "Shorter" }),
            ),
        )
        .await
        .unwrap();
        assert_eq!(out["version"], 2);
        let r = f.store.revision(MARK, id, 2).await.unwrap().unwrap();
        assert_eq!(r.author, Author::Chatgpt);
        assert_eq!(r.reason, "Shorter");
    }

    #[tokio::test]
    async fn a_turn_carries_what_it_made_and_its_conversation_is_kept_for_its_jobs() {
        let f = fixture(FakeRunner::doing(vec![]));
        let before = Instant::now();
        let id = call(
            &f,
            asking(MARK, None),
            "artifact_create",
            json!({ "kind": "document", "title": "Notes", "summary": "s", "content": "- one" }),
        )
        .await
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        let cards = f.artifacts.for_turn(MARK, before).await;
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].title, "Notes");
        assert_eq!(cards[0].preview.lines[0].text, "one");
        assert!(f.artifacts.for_turn(MAJSE, before).await.is_empty());
        assert!(f.artifacts.for_turn(MARK, Instant::now()).await.is_empty());

        f.artifacts.placed(MARK, before, 42).await;
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!(a.conversation_id, Some(42));

        let json = serde_json::to_value(&cards[0]).unwrap();
        assert_eq!(json["status"], "ready");
        assert_eq!(json["preview"]["lines"][0]["style"], "bullet");
    }

    #[tokio::test]
    async fn cards_read_back_are_brought_up_to_date() {
        let f = fixture(FakeRunner::doing(vec![]));
        let kept = create(&f, MARK, "Packing list").await;
        let gone = create(&f, MARK, "Old notes").await;
        let cards = vec![
            ArtifactRef::of(&f.store.get(MARK, kept).await.unwrap().unwrap()),
            ArtifactRef::of(&f.store.get(MARK, gone).await.unwrap().unwrap()),
        ];
        f.store
            .change(
                MARK,
                kept,
                niles_artifacts::Changes {
                    title: Some("Cabin packing".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        f.store.delete(MARK, gone).await.unwrap();
        let mut turns = vec![Turn {
            at: chrono::Utc::now(),
            said: "make a packing list".into(),
            reply: "Saved.".into(),
            via: "app".into(),
            room: None,
            by: Some("claude".into()),
            proposal: None,
            approval: None,
            artifacts: cards,
            attachments: Vec::new(),
        }];
        f.artifacts.refresh(MARK, &mut turns).await;
        assert_eq!(turns[0].artifacts.len(), 1);
        assert_eq!(turns[0].artifacts[0].title, "Cabin packing");

        // Somebody else reading the same turns finds none of it.
        f.artifacts.refresh(MAJSE, &mut turns).await;
        assert!(turns[0].artifacts.is_empty());
    }

    #[tokio::test]
    async fn a_job_is_working_then_ready_with_a_card_in_its_chat() {
        let f = fixture(FakeRunner::doing(vec![Does::Saves(
            "# Thailand\n\n- Bangkok\n- Koh Tao",
        )]));
        let mut events = f.bus.subscribe();
        let out = start(&f, MARK, Some(7)).await.unwrap();
        assert_eq!(out["status"], "working");
        assert_eq!(out["status_text"], "Writing your Thailand travel plan…");
        let id = out["id"].as_i64().unwrap();

        let done = settled(&f, MARK, id).await;
        assert_eq!(done.status, Status::Ready);
        assert_eq!(done.status_text, None);
        assert_eq!(done.current_revision, 1);
        assert_eq!(done.preview.lines[0].text, "Thailand");

        let statuses: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|e| match e {
                Event::ArtifactChanged { person, status, .. } => {
                    assert_eq!(person, MARK);
                    Some(status)
                }
                _ => None,
            })
            .collect();
        assert_eq!(statuses, vec!["working", "ready"]);

        let chats = f.chats.0.lock().unwrap();
        let (person, conversation, turn) = &chats[0];
        assert_eq!((person.as_str(), *conversation), (MARK, 7));
        assert_eq!(turn.reply, "Thailand travel plan is ready.");
        assert_eq!(turn.by.as_deref(), Some("claude"));
        assert_eq!(turn.artifacts[0].id, id);
        assert_eq!(turn.artifacts[0].status, Status::Ready);
    }

    #[tokio::test]
    async fn a_job_that_answers_with_the_document_keeps_it() {
        let f = fixture(FakeRunner::doing(vec![Does::Answers(
            "# Packing\n\n- passport\n- charger\n- sunscreen",
        )]));
        let id = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        let done = settled(&f, MARK, id).await;
        assert_eq!(done.status, Status::Ready);
        let r = f.store.revision(MARK, id, 1).await.unwrap().unwrap();
        assert_eq!(r.author, Author::Claude);
        assert!(
            r.content["markdown"]
                .as_str()
                .unwrap()
                .contains("sunscreen")
        );
    }

    #[tokio::test]
    async fn a_job_that_saves_nothing_has_failed() {
        let f = fixture(FakeRunner::doing(vec![Does::Answers("Done.")]));
        let id = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        let done = settled(&f, MARK, id).await;
        assert_eq!(done.status, Status::Failed);
        assert!(done.error.unwrap().contains("without saving"));
    }

    #[tokio::test]
    async fn a_failed_job_says_why_and_can_be_tried_again() {
        let f = fixture(FakeRunner::doing(vec![
            Does::Fails("Claude Code did not finish: no answer within 1200s"),
            Does::Saves("# Thailand\n\n- Bangkok"),
        ]));
        let id = start(&f, MARK, Some(3)).await.unwrap()["id"]
            .as_i64()
            .unwrap();
        let failed = settled(&f, MARK, id).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(failed.error.as_deref().unwrap().contains("1200s"));
        assert!(
            f.chats.0.lock().unwrap().is_empty(),
            "no card for a failure"
        );

        let again = f.artifacts.retry(MARK, id).await.unwrap();
        assert_eq!(again.status, Status::Working);
        assert_eq!(
            again.status_text.as_deref(),
            Some("Writing your Thailand travel plan…")
        );
        let done = settled(&f, MARK, id).await;
        assert_eq!(done.status, Status::Ready);
        assert_eq!(done.error, None);
        assert_eq!(done.brief.as_deref(), failed.brief.as_deref());
    }

    #[tokio::test]
    async fn only_a_failed_job_of_theirs_is_tried_again() {
        use niles_api::artifacts::Refused;
        let f = fixture(FakeRunner::doing(vec![]));
        let ready = create(&f, MARK, "Packing list").await;
        assert!(matches!(
            f.artifacts.retry(MARK, ready).await,
            Err(Refused::NotRetryable(_))
        ));
        assert_eq!(
            f.artifacts.retry(MAJSE, ready).await.unwrap_err(),
            Refused::NotFound
        );
    }

    #[tokio::test]
    async fn two_jobs_at_a_time_per_person() {
        let (first, second) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let f = fixture(FakeRunner::doing(vec![
            Does::Waits(first.clone()),
            Does::Waits(second.clone()),
            Does::Saves("# third\n\n- a\n- b"),
            Does::Saves("# hers\n\n- a\n- b"),
        ]));
        let a = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        let b = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        let e = start(&f, MARK, None).await.unwrap_err().to_string();
        assert!(e.contains("already"), "{e}");
        assert_eq!(
            f.store.list(MARK, Filter::default()).await.unwrap().len(),
            2
        );

        // Somebody else's are counted apart.
        let hers = start(&f, MAJSE, None).await.unwrap()["id"]
            .as_i64()
            .unwrap();
        let _ = hers;

        first.notify_one();
        settled(&f, MARK, a).await;
        let c = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        second.notify_one();
        settled(&f, MARK, b).await;
        settled(&f, MARK, c).await;
    }

    #[tokio::test]
    async fn a_job_cannot_write_any_artifact_but_its_own() {
        let f = fixture(FakeRunner::doing(vec![]));
        let other = create(&f, MARK, "Packing list").await;
        let tools = f.artifacts.job_tools(other + 100, "document", None);
        let e = approvals::ASKER
            .scope(
                asking(MARK, None),
                tools.execute(&niles_llm::ToolCall {
                    id: String::new(),
                    name: "artifact_update".into(),
                    arguments: json!({ "id": other, "content": "x", "reason": "y" }),
                }),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("only that one"), "{e}");
        assert!(!tools.has("start_artifact_job"));
        assert!(!tools.has("artifact_create"));
    }

    #[tokio::test]
    async fn only_a_training_plan_s_job_reads_the_person_s_workouts() {
        let store = Arc::new(MemoryArtifacts::new());
        let artifacts = Arc::new(
            Artifacts::new(
                store,
                EventBus::default(),
                FakeRunner::doing(vec![]),
                Arc::new(Nobody),
                None,
                Box::new(|_| {}),
            )
            .with_training(
                Box::new(|reg: &mut ToolRegistry| {
                    crate::training::register(
                        reg,
                        Arc::new(niles_training::MemoryTraining::new()),
                        chrono_tz::UTC,
                        false,
                    );
                }),
                chrono_tz::UTC,
            ),
        );
        let plan = artifacts.job_tools(1, "training_plan", None);
        assert!(plan.has("training_summary"));
        assert!(plan.has("training_activities"));
        assert!(plan.has("artifact_update"));
        let document = artifacts.job_tools(2, "document", None);
        assert!(!document.has("training_summary"));
    }

    fn working(kind: &str) -> Artifact {
        let now = chrono::Utc::now();
        Artifact {
            id: 7,
            person: MAJSE.into(),
            kind: kind.into(),
            title: "Half marathon plan".into(),
            summary: "Twelve weeks to the race".into(),
            pinned: false,
            archived: false,
            notify: true,
            status: Status::Working,
            status_text: None,
            error: None,
            job_id: Some("j".into()),
            brief: Some("Copenhagen Half on 13 December, four runs a week".into()),
            conversation_id: None,
            created_at: now,
            updated_at: now,
            current_revision: 0,
            preview: Default::default(),
            page: None,
        }
    }

    #[test]
    fn a_training_plan_s_job_is_told_to_build_on_her_running() {
        let (system, prompt) = job_prompts(&working("training_plan"));
        assert!(system.contains("writing a training plan"), "{system}");
        assert!(system.contains("training_summary"), "{system}");
        assert!(system.contains("10%"), "{system}");
        assert!(
            system.contains("\"weeks\""),
            "the content's shape: {system}"
        );
        assert!(!system.contains("ferries"), "{system}");
        assert!(prompt.contains("Copenhagen Half"), "{prompt}");

        let (system, _) = job_prompts(&working("document"));
        assert!(system.contains("writing a document"), "{system}");
        assert!(system.contains("web_search"), "{system}");
        assert!(!system.contains("training_summary"), "{system}");
    }

    #[test]
    fn a_trip_s_job_researches_it_and_books_nothing() {
        let (system, _) = job_prompts(&working("trip"));
        assert!(system.contains("writing a trip"), "{system}");
        assert!(system.contains("web_search"), "{system}");
        assert!(system.contains("Mark nothing as booked"), "{system}");
        assert!(system.contains("packing list"), "{system}");
        assert!(system.contains("\"days\""), "the content's shape: {system}");
        assert!(!system.contains("training_summary"), "{system}");
    }

    #[test]
    fn the_chat_is_told_a_travel_plan_is_a_trip() {
        assert!(PROMPT.contains("itinerary is kind trip"), "{PROMPT}");
    }

    #[tokio::test]
    async fn a_training_plan_that_is_wrong_says_what_to_fix() {
        let f = fixture(FakeRunner::doing(vec![]));
        let plan = |starts: &str| {
            json!({
                "kind": "training_plan",
                "title": "Half marathon plan",
                "summary": "Toward the Copenhagen Half",
                "content": {
                    "goal": { "race": "Copenhagen Half", "distance_km": 21.1, "date": "2026-12-13" },
                    "weeks": [
                        { "starts": "2026-10-19", "sessions": [
                            { "date": "2026-10-20", "type": "easy", "title": "Easy run", "distance_km": 6 }
                        ]},
                        { "starts": starts, "sessions": [] }
                    ]
                }
            })
        };
        let e = call(
            &f,
            asking(MAJSE, None),
            "artifact_create",
            plan("2026-10-27"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("must start on 2026-10-26"), "{e}");

        let made = call(
            &f,
            asking(MAJSE, None),
            "artifact_create",
            plan("2026-10-26"),
        )
        .await
        .unwrap();
        assert_eq!(made["kind"], "training_plan");
        assert_eq!(
            made["preview"],
            json!([
                "Copenhagen Half · Sun 13 Dec · 2 weeks",
                "Tue 20 Oct · Easy run · 6 km"
            ])
        );
    }

    #[tokio::test]
    async fn without_an_assistant_no_job_starts() {
        let f = fixture(Arc::new(FakeRunner {
            on: false,
            jobs: Mutex::default(),
        }));
        let e = start(&f, MARK, None).await.unwrap_err().to_string();
        assert!(e.contains("Voice & AI"), "{e}");
        assert!(
            f.store
                .list(MARK, Filter::default())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_person_is_told_when_notify_is_on_and_not_when_it_is_off() {
        let f = fixture(FakeRunner::doing(vec![
            Does::Saves("# on\n\n- a"),
            Does::Saves("# off\n\n- a"),
        ]));
        let on = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        settled(&f, MARK, on).await;
        let off = call(
            &f,
            asking(MARK, None),
            "start_artifact_job",
            json!({ "kind": "document", "title": "Quiet one", "summary": "s",
                    "brief": "b", "notify": false }),
        )
        .await
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        settled(&f, MARK, off).await;
        let told = f.told.0.lock().unwrap();
        assert_eq!(*told, vec![(on, Status::Ready)]);
    }

    #[tokio::test]
    async fn a_failure_is_told_too_when_notify_is_on() {
        let f = fixture(FakeRunner::doing(vec![Does::Fails("no")]));
        let id = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        settled(&f, MARK, id).await;
        assert_eq!(*f.told.0.lock().unwrap(), vec![(id, Status::Failed)]);
    }

    #[tokio::test]
    async fn the_notification_names_the_artifact_and_opens_it() {
        let f = fixture(FakeRunner::doing(vec![Does::Saves("# Plan\n\n- Bangkok")]));
        let id = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        settled(&f, MARK, id).await;
        let artifact = f.store.get(MARK, id).await.unwrap().unwrap();
        let message = finished_message(&artifact);
        assert_eq!(message.title, "Niles");
        assert_eq!(message.body, "Your Thailand travel plan is ready.");
        assert_eq!(message.url, format!("/#/chat?artifact={id}"));
        assert_eq!(message.tag, format!("artifact-{id}"));
    }

    #[tokio::test]
    async fn a_failed_job_s_notification_says_so_under_the_same_tag() {
        let f = fixture(FakeRunner::doing(vec![Does::Fails("no")]));
        let id = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        settled(&f, MARK, id).await;
        let artifact = f.store.get(MARK, id).await.unwrap().unwrap();
        let message = finished_message(&artifact);
        assert!(message.body.contains("didn't finish"), "{}", message.body);
        assert_eq!(message.tag, format!("artifact-{id}"));
    }

    #[test]
    fn a_plan_the_coach_changed_says_so_with_its_note() {
        let plan = working("training_plan");
        let message = changed_message(&plan, "A little less next week.");
        assert_eq!(
            message.body,
            "Your Half marathon plan changed after your run. A little less next week."
        );
        assert_eq!(message.url, "/#/chat?artifact=7");
        assert_eq!(message.tag, "artifact-7");
        let long = changed_message(&plan, &"word ".repeat(100));
        assert!(long.body.ends_with('…'), "{}", long.body);
        assert!(long.body.chars().count() < 300);
    }

    const PAGE: &str =
        "<ul id=l></ul><script>niles.onData(function(d){l.textContent=d.markdown})</script>";

    #[tokio::test]
    async fn a_page_is_set_read_back_kept_through_an_update_and_taken_away() {
        let f = fixture(FakeRunner::doing(vec![]));
        let id = create(&f, MARK, "Packing list").await;
        let set = call(
            &f,
            asking(MARK, None),
            "artifact_set_page",
            json!({ "id": id, "html": PAGE, "reason": "A page with a list" }),
        )
        .await
        .unwrap();
        assert_eq!(set["page"], true);
        assert_eq!(set["version"], 2);
        let r = f.store.revision(MARK, id, 2).await.unwrap().unwrap();
        assert_eq!(r.reason, "A page with a list");
        assert_eq!(r.content["markdown"], "- [ ] passport\n- [ ] charger");

        let got = call(&f, asking(MARK, None), "artifact_get", json!({ "id": id }))
            .await
            .unwrap();
        assert_eq!(got["page"], true);
        assert!(got.get("page_html").is_none(), "only when asked for");
        let got = call(
            &f,
            asking(MARK, None),
            "artifact_get",
            json!({ "id": id, "with_page": true }),
        )
        .await
        .unwrap();
        assert_eq!(got["page_html"], PAGE);

        let updated = call(
            &f,
            asking(MARK, None),
            "artifact_update",
            json!({ "id": id, "content": { "markdown": "- [ ] passport" }, "reason": "Shorter" }),
        )
        .await
        .unwrap();
        assert_eq!(updated["page"], true, "a change to the data keeps the page");

        let removed = call(
            &f,
            asking(MARK, None),
            "artifact_set_page",
            json!({ "id": id, "remove": true, "reason": "" }),
        )
        .await
        .unwrap();
        assert_eq!(removed["page"], false);
        let r = f.store.revision(MARK, id, 4).await.unwrap().unwrap();
        assert_eq!(r.reason, "Took the page away");
        assert_eq!(r.content["markdown"], "- [ ] passport");
    }

    #[tokio::test]
    async fn a_page_from_elsewhere_too_big_or_for_someone_else_is_refused() {
        let f = fixture(FakeRunner::doing(vec![]));
        let id = create(&f, MARK, "Packing list").await;
        let set = |html: String, person: &'static str| {
            call(
                &f,
                asking(person, None),
                "artifact_set_page",
                json!({ "id": id, "html": html, "reason": "x" }),
            )
        };
        let e = set(
            "<script src=\"https://cdn.example/x.js\"></script>".into(),
            MARK,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("frame blocks"), "{e}");
        let e = set(format!("<p>{}</p>", "x".repeat(page::MAX_BYTES)), MARK)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("200 KB"), "{e}");
        assert!(set(PAGE.into(), MAJSE).await.is_err());
        let e = call(
            &f,
            asking(MARK, None),
            "artifact_set_page",
            json!({ "id": id, "reason": "x" }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("remove: true"), "{e}");
        let still = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!((still.current_revision, still.page), (1, None));
    }

    #[tokio::test]
    async fn a_job_may_give_its_own_artifact_a_page_and_only_its_own() {
        let f = fixture(FakeRunner::doing(vec![]));
        let other = create(&f, MARK, "Packing list").await;
        let tools = f.artifacts.job_tools(other + 100, "document", None);
        assert!(tools.has("artifact_set_page"));
        let e = approvals::ASKER
            .scope(
                asking(MARK, None),
                tools.execute(&niles_llm::ToolCall {
                    id: String::new(),
                    name: "artifact_set_page".into(),
                    arguments: json!({ "id": other, "html": PAGE, "reason": "y" }),
                }),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("only that one"), "{e}");
        assert!(!tools.has("start_page_job"));
    }

    /// Until no page is being made for `id`, or two seconds pass.
    async fn paged(f: &Fixture, id: i64) {
        for _ in 0..200 {
            if !f.artifacts.paging.lock().unwrap().contains(&id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("a page was still being made for {id}");
    }

    async fn start_page(f: &Fixture, person: &str, id: i64) -> Result<Value> {
        call(
            f,
            asking(person, Some(9)),
            "start_page_job",
            json!({ "id": id, "brief": "A clean list with big boxes" }),
        )
        .await
    }

    #[tokio::test]
    async fn a_page_job_makes_the_page_and_says_so_in_its_chat() {
        let f = fixture(FakeRunner::doing(vec![Does::Pages(PAGE)]));
        let id = create(&f, MARK, "Packing list").await;
        let out = start_page(&f, MARK, id).await.unwrap();
        assert_eq!(out["status"], "ready", "the artifact is not marked working");
        paged(&f, id).await;
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!(a.status, Status::Ready);
        let html = f.store.page(MARK, id, a.page.unwrap()).await.unwrap();
        assert_eq!(html.as_deref(), Some(PAGE));
        let chats = f.chats.0.lock().unwrap();
        let (_, conversation, turn) = &chats[0];
        assert_eq!(*conversation, 9);
        assert_eq!(turn.reply, "The page for Packing list is ready.");
        assert_eq!(*f.told.1.lock().unwrap(), vec![id]);
        assert!(
            f.told.0.lock().unwrap().is_empty(),
            "not ready all over again"
        );
    }

    #[tokio::test]
    async fn a_page_job_that_answers_with_the_page_keeps_it() {
        let f = fixture(FakeRunner::doing(vec![Does::Answers(
            "Here it is:\n```html\n<h1 id=t></h1><script>niles.onData(function(d){})</script>\n```",
        )]));
        let id = create(&f, MARK, "Packing list").await;
        start_page(&f, MARK, id).await.unwrap();
        paged(&f, id).await;
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        let html = f.store.page(MARK, id, a.page.unwrap()).await.unwrap();
        assert!(html.unwrap().starts_with("<h1 id=t>"));
        let r = f
            .store
            .revision(MARK, id, a.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.author, Author::Claude);
    }

    #[tokio::test]
    async fn a_page_job_that_makes_nothing_says_why_and_changes_nothing() {
        let f = fixture(FakeRunner::doing(vec![Does::Answers("Done.")]));
        let id = create(&f, MARK, "Packing list").await;
        start_page(&f, MARK, id).await.unwrap();
        paged(&f, id).await;
        let a = f.store.get(MARK, id).await.unwrap().unwrap();
        assert_eq!((a.current_revision, a.page), (1, None));
        let chats = f.chats.0.lock().unwrap();
        assert!(
            chats[0].2.reply.contains("didn't get made"),
            "{}",
            chats[0].2.reply
        );
        assert!(f.told.1.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_page_job_at_a_time_and_none_for_what_is_being_written() {
        let (writing, paging) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let f = fixture(FakeRunner::doing(vec![
            Does::Waits(writing.clone()),
            Does::Waits(paging.clone()),
        ]));
        let job = start(&f, MARK, None).await.unwrap()["id"].as_i64().unwrap();
        let e = start_page(&f, MARK, job).await.unwrap_err().to_string();
        assert!(e.contains("still being written"), "{e}");
        writing.notify_one();
        settled(&f, MARK, job).await;

        let id = create(&f, MARK, "Packing list").await;
        start_page(&f, MARK, id).await.unwrap();
        let e = start_page(&f, MARK, id).await.unwrap_err().to_string();
        assert!(e.contains("being made for it already"), "{e}");
        let e = start_page(&f, MAJSE, id).await.unwrap_err().to_string();
        assert!(e.contains("no artifact"), "{e}");
        paging.notify_one();
        paged(&f, id).await;
    }

    #[test]
    fn a_page_job_is_told_the_page_api_and_the_chat_when_to_make_one() {
        let (system, prompt) =
            page_prompts(&working("training_plan"), "Make it look like a race poster");
        assert!(
            system.contains("artifact_set_page on artifact 7"),
            "{system}"
        );
        assert!(system.contains("niles.onData"), "{system}");
        assert!(system.contains("never to contain it"), "{system}");
        assert!(system.contains("\"weeks\""), "the data's shape: {system}");
        assert!(prompt.contains("race poster"), "{prompt}");
        assert!(PROMPT.contains("start_page_job"), "{PROMPT}");
        assert!(PROMPT.contains("make no page unasked"), "{PROMPT}");
        let (system, _) = job_prompts(&working("trip"));
        assert!(system.contains("Only if the brief asks"), "{system}");
    }

    #[test]
    fn a_page_s_notification_names_the_artifact_under_its_tag() {
        let message = page_message(&working("training_plan"));
        assert_eq!(
            message.body,
            "The page for your Half marathon plan is ready."
        );
        assert_eq!(message.tag, "artifact-7");
    }

    #[tokio::test]
    async fn a_job_deleted_while_it_runs_is_simply_dropped() {
        let go = Arc::new(Notify::new());
        let f = fixture(FakeRunner::doing(vec![Does::Waits(go.clone())]));
        let id = start(&f, MARK, Some(1)).await.unwrap()["id"]
            .as_i64()
            .unwrap();
        f.store.delete(MARK, id).await.unwrap();
        go.notify_one();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(f.chats.0.lock().unwrap().is_empty());
        assert!(f.told.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_restart_fails_what_the_last_run_was_writing_and_not_its_own() {
        let store = Arc::new(MemoryArtifacts::new());
        let left = store
            .create(
                MARK,
                NewArtifact {
                    kind: "document".into(),
                    title: "Half written".into(),
                    summary: String::new(),
                    notify: true,
                    status: Status::Working,
                    status_text: Some("Writing…".into()),
                    job_id: Some("old".into()),
                    brief: Some("b".into()),
                    conversation_id: None,
                    first: None,
                },
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let go = Arc::new(Notify::new());
        let artifacts = Arc::new(Artifacts::new(
            store.clone(),
            EventBus::default(),
            FakeRunner::doing(vec![Does::Waits(go)]),
            Arc::new(Nobody),
            None,
            Box::new(|_| {}),
        ));
        let own = artifacts
            .start(
                &asking(MARK, None),
                Job {
                    kind: kinds::require("document").unwrap(),
                    title: "New".into(),
                    summary: String::new(),
                    brief: "b".into(),
                    status_text: "Writing…".into(),
                    notify: true,
                },
            )
            .await
            .unwrap();
        artifacts.recover().await;
        let left = store.get(MARK, left.id).await.unwrap().unwrap();
        assert_eq!(left.status, Status::Failed);
        assert_eq!(left.error.as_deref(), Some(RESTARTED));
        let own = store.get(MARK, own.id).await.unwrap().unwrap();
        assert_eq!(own.status, Status::Working);
    }
}

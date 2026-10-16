//! Artifacts: what Claude or ChatGPT makes for one person and keeps for
//! them. See ARCHITECTURE.md § Artifacts (per person).
//!
//! The tools here are offered to the assistant alone (see
//! [`crate::assistant::Assistant::start`]), and act for the asker of
//! the turn, [`approvals::ASKER`]: whose artifacts these are is never an
//! argument. A big one is a job: the artifact is made at once, marked
//! working, and a background run of the same assistant writes it while
//! the chat goes on.

use crate::approvals::{self, Asker};
use async_trait::async_trait;
use niles_api::chat::Turn;
use niles_artifacts::{
    Artifact, ArtifactRef, ArtifactStore, Author, Filter, NewArtifact, NewRevision, Status,
    StatusChange, kinds,
};
use niles_core::EventBus;
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::collections::HashMap;
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

/// Told when a job ends, if the artifact's notify is on. Nothing is
/// behind it yet: phone push (Web Push) comes as its own piece of work.
#[async_trait]
pub(crate) trait Notifier: Send + Sync {
    async fn finished(&self, artifact: &Artifact);
}

/// Tells nobody. The person still sees the status line, the Saved tab
/// and the card in the chat.
pub(crate) struct Nobody;

#[async_trait]
impl Notifier for Nobody {
    async fn finished(&self, _artifact: &Artifact) {}
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
            crate::home_context(&self.settings.current().home)
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
            running: Arc::default(),
            touched: Mutex::default(),
            started: chrono::Utc::now(),
        }
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
        }));
        reg.register(Box::new(StartJob(self.clone())));
        reg
    }

    /// What a job's run may use: its person's artifacts to read, its own
    /// to write, and what writing needs. Not the house.
    fn job_tools(self: &Arc<Self>, id: i64) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(List(self.clone())));
        reg.register(Box::new(Get(self.clone())));
        reg.register(Box::new(Update {
            artifacts: self.clone(),
            only: Some(id),
        }));
        (self.extras)(&mut reg);
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
        let tools = self.job_tools(artifact.id);
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
            self.tell_in_chat(&done, via).await;
        }
        if done.notify {
            self.notifier.finished(&done).await;
        }
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
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A card in the conversation the job came from, saying it is ready.
    async fn tell_in_chat(&self, artifact: &Artifact, via: &'static str) {
        let (Some(conversations), Some(conversation)) =
            (&self.conversations, artifact.conversation_id)
        else {
            return;
        };
        let turn = Turn {
            at: chrono::Utc::now(),
            said: String::new(),
            reply: format!("{} is ready.", artifact.title),
            via: "app".into(),
            room: None,
            by: Some(via.to_string()),
            proposal: None,
            approval: None,
            artifacts: vec![ArtifactRef::of(artifact)],
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

/// The run's instructions, and what it is to make.
fn job_prompts(artifact: &Artifact) -> (String, String) {
    let kind = kinds::kind(&artifact.kind);
    let label = kind.map_or("document", |k| k.label);
    let help = kind.map_or("", |k| k.content_help);
    let system = format!(
        "You are writing a {label} for one person in the household Niles serves, in the \
         background. Nobody is waiting on you in a chat: take the time to do it well, and \
         ask nothing, because nobody will answer. When it is written, save it with \
         artifact_update on artifact {id}, the whole content, with the reason \"First \
         draft\" and a one-line summary. Then finish with one short sentence saying what you \
         wrote.\n\nThe content is {help}\n\nWhere facts matter (opening times, ferries, \
         prices, the weather in a season), check them with web_search and read_page when you \
         have them, and say in the document what you could not check. Write in the language \
         the brief is in.",
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
    will find it under Saved. Do not write it out in the chat as well.";

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
                content. Read it before changing it: they may have ticked items since."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } },
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
                    }),
                },
            )
            .await
            .map_err(|e| refused(TOOL, e))?;
        self.0.touch(&asker.person, a.id);
        self.0.announce(&a);
        Ok(view(&a))
    }
}

struct Update {
    artifacts: Arc<Artifacts>,
    /// In a job, the one artifact it may write.
    only: Option<i64>,
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
        let reason = niles_artifacts::summary(str_arg(&args, "reason").unwrap_or("Changed"));
        let a = store
            .revise(
                &asker.person,
                id,
                NewRevision {
                    content,
                    reason,
                    author: author(crate::assistant::via()),
                    summary: str_arg(&args, "summary").map(niles_artifacts::summary),
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
        Ok(view(&a))
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
            }
        }
    }

    #[derive(Default)]
    struct Told(Mutex<Vec<(i64, Status)>>);

    #[async_trait]
    impl Notifier for Told {
        async fn finished(&self, artifact: &Artifact) {
            self.0.lock().unwrap().push((artifact.id, artifact.status));
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
        let tools = f.artifacts.job_tools(other + 100);
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

//! Approvals: what Niles holds for a person's yes instead of doing on
//! the model's word. See ARCHITECTURE.md's House rules.
//!
//! A held call keeps the tool and its arguments exactly as the model
//! sent them. The person decides — a button on the card in the app or
//! in Niles's panel, or a spoken yes at the satellite that asked — and
//! Niles runs what it held, never what the model says next.
//!
//! Held calls are kept in memory. A restart lapses them; a card still
//! pending in a conversation then reads as lapsed. What was asked, and
//! what became of it, is kept in the approval log: the card's title and
//! one summary line of it, never the arguments.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_api::approvals::{Decided, History, Waiting};
use niles_api::chat::{ApprovalCard, ApprovalStatus, CardField, Decision, Undecidable};
use niles_core::{Event, EventBus};
use niles_db::{ApprovalEntry, ApprovalLogged};
use niles_tools::{Tool, ToolDescriptor};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a question asked aloud waits for its yes.
const BY_VOICE: Duration = Duration::from_secs(5 * 60);
/// How long a card in the app waits: long enough to be answered from
/// the phone after it was asked for in the kitchen.
const IN_THE_APP: Duration = Duration::from_secs(60 * 60);
/// What a card's value is cut to; the rest is behind the chevron in the
/// app, and there is no chevron in a sentence.
const MAX_FIELD: usize = 2_000;
/// The one line a card is remembered by, in the panel and the log.
const SUMMARY_CHARS: usize = 200;
/// How often what lapsed is noticed, said and logged.
const SWEEP_EVERY: Duration = Duration::from_secs(30);
/// One page of history.
const PAGE: usize = 30;

/// Who a turn is for, and where they are: what a held call is kept
/// against, and who may answer it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asker {
    /// The chat log's key for them: `person:<slug>`, `email:<address>`,
    /// or `satellite:<address>` for a voice nobody recognised.
    pub(crate) person: String,
    /// Their voice, when it is known.
    pub(crate) slug: Option<String>,
    /// The satellite a spoken turn came from. `None` in the app.
    pub(crate) satellite: Option<SocketAddr>,
    /// The app conversation, once it has one.
    pub(crate) conversation: Option<i64>,
}

tokio::task_local! {
    /// Set for the length of a turn, and for an assistant's tool calls
    /// made on that turn's behalf.
    pub(crate) static ASKER: Asker;
}

/// The asker of the turn this runs in, if any.
pub(crate) fn asker() -> Option<Asker> {
    ASKER.try_with(Clone::clone).ok()
}

/// Where somebody answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    /// A button: on the card in the chat, or in Niles's panel.
    App,
    /// A yes or a no at the satellite that asked.
    Voice,
}

impl Via {
    fn as_str(self) -> &'static str {
        match self {
            Via::App => "app",
            Via::Voice => "voice",
        }
    }
}

/// Where what was asked and decided is kept. Postgres in the house; a
/// list in the tests.
#[async_trait]
pub(crate) trait Ledger: Send + Sync {
    async fn asked(&self, entry: &ApprovalEntry) -> Result<(), String>;
    async fn settled(&self, entry: &ApprovalEntry) -> Result<(), String>;
    async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<ApprovalLogged>, String>;
}

#[async_trait]
impl Ledger for niles_db::PostgresApprovalLog {
    async fn asked(&self, entry: &ApprovalEntry) -> Result<(), String> {
        niles_db::PostgresApprovalLog::asked(self, entry).await
    }

    async fn settled(&self, entry: &ApprovalEntry) -> Result<(), String> {
        niles_db::PostgresApprovalLog::settled(self, entry).await
    }

    async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<ApprovalLogged>, String> {
        niles_db::PostgresApprovalLog::page(self, person, before, limit).await
    }
}

pub(crate) struct Held {
    asker: Asker,
    tool: Arc<dyn Tool>,
    name: String,
    /// The add-on it is for, as a person reads it.
    label: String,
    args: Value,
    pub(crate) card: ApprovalCard,
    made: Instant,
    lapses: Instant,
    asked_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// Where a held call's card is, for answering it by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placed {
    /// The conversation its card is in; none for a question asked aloud.
    pub(crate) conversation: Option<i64>,
    pub(crate) allow_conversation: bool,
}

#[derive(Default)]
pub(crate) struct Approvals {
    held: Mutex<HashMap<String, Held>>,
    /// "Allow for this conversation": (person, conversation, tool).
    grants: Mutex<HashSet<(String, i64, String)>>,
    /// Where each person hears that something waits, or no longer does.
    bus: EventBus,
    /// What was asked and decided. None without a database.
    ledger: Option<Arc<dyn Ledger>>,
}

impl Approvals {
    pub(crate) fn new(bus: EventBus, ledger: Option<Arc<dyn Ledger>>) -> Self {
        Self {
            bus,
            ledger,
            ..Self::default()
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keep `tool` called with `args` for `asker`'s yes. `label` names
    /// what it is for: "Linear", "Mail". Answers the card as it is to be
    /// shown.
    pub(crate) fn hold(
        &self,
        asker: &Asker,
        tool: Arc<dyn Tool>,
        label: &str,
        args: Value,
        mut card: ApprovalCard,
    ) -> ApprovalCard {
        let now = Instant::now();
        let wait = if asker.satellite.is_some() {
            BY_VOICE
        } else {
            IN_THE_APP
        };
        card.id = new_id();
        card.status = ApprovalStatus::Pending;
        card.outcome = None;
        if asker.satellite.is_some() {
            card.allow_conversation = false;
        }
        let asked_at = Utc::now();
        let held = Held {
            asker: asker.clone(),
            name: tool.descriptor().name,
            tool,
            label: label.to_string(),
            args,
            card: card.clone(),
            made: now,
            lapses: now + wait,
            asked_at,
            expires_at: asked_at + chrono::Duration::from_std(wait).unwrap_or_default(),
        };
        if let Some(ledger) = self.ledger.clone() {
            let entry = entry(&held, &card);
            tokio::spawn(async move {
                if let Err(e) = ledger.asked(&entry).await {
                    tracing::warn!("[approvals] what was asked was not logged: {e}");
                }
            });
        }
        self.lock().insert(card.id.clone(), held);
        self.changed(&asker.person, &card);
        card
    }

    /// The card a turn of `person`'s that began at `since` left waiting
    /// in the app, to keep with the turn.
    pub(crate) fn for_turn(&self, person: &str, since: Instant) -> Option<ApprovalCard> {
        self.lock()
            .values()
            .filter(|h| h.asker.person == person && h.asker.satellite.is_none() && h.made >= since)
            .max_by_key(|h| h.made)
            .map(|h| h.card.clone())
    }

    /// Note the conversation a card went into, once the turn is kept.
    pub(crate) fn place(&self, id: &str, conversation: i64) {
        if let Some(h) = self.lock().get_mut(id) {
            h.asker.conversation = Some(conversation);
        }
    }

    /// Whether a card is still waiting.
    pub(crate) fn waiting(&self, id: &str) -> bool {
        self.lock()
            .get(id)
            .is_some_and(|h| h.lapses > Instant::now())
    }

    /// Everything still waiting for `person`, newest first: what the
    /// panel lists. Nobody else's, by voice or in the app.
    pub(crate) fn pending(&self, person: &str) -> Vec<Waiting> {
        let now = Instant::now();
        let held = self.lock();
        let mut theirs: Vec<&Held> = held
            .values()
            .filter(|h| h.asker.person == person && h.lapses > now)
            .collect();
        theirs.sort_by_key(|h| std::cmp::Reverse(h.made));
        theirs
            .into_iter()
            .map(|h| Waiting {
                card: h.card.clone(),
                tool: h.label.clone(),
                summary: summary(&h.card),
                conversation_id: h.asker.conversation,
                asked_at: h.asked_at,
                expires_at: h.expires_at,
                via: asked_via(&h.asker).as_str().into(),
            })
            .collect()
    }

    /// Where `person`'s held call `id` is, to answer it from the panel.
    /// Somebody else's is no such approval, the same as one never held.
    pub(crate) fn placed(&self, id: &str, person: &str) -> Result<Placed, Undecidable> {
        let held = self.lock();
        let h = held
            .get(id)
            .filter(|h| h.asker.person == person)
            .ok_or(Undecidable::NoSuchApproval)?;
        if h.lapses <= Instant::now() {
            return Err(Undecidable::NotWaiting);
        }
        Ok(Placed {
            conversation: h.asker.conversation,
            allow_conversation: h.card.allow_conversation,
        })
    }

    /// Take a held call to run or drop, if it is `person`'s and still
    /// waiting.
    pub(crate) fn take(&self, id: &str, person: &str) -> Option<Held> {
        let mut held = self.lock();
        let ok = held
            .get(id)
            .is_some_and(|h| h.asker.person == person && h.lapses > Instant::now());
        if ok { held.remove(id) } else { None }
    }

    /// The question asked aloud at `satellite` that is still waiting, if
    /// one is: the newest.
    pub(crate) fn spoken(&self, satellite: SocketAddr) -> Option<(String, Asker)> {
        let now = Instant::now();
        self.lock()
            .values()
            .filter(|h| h.asker.satellite == Some(satellite) && h.lapses > now)
            .max_by_key(|h| h.made)
            .map(|h| (h.card.id.clone(), h.asker.clone()))
    }

    pub(crate) fn grant(&self, person: &str, conversation: i64, tool: &str) {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((person.to_string(), conversation, tool.to_string()));
    }

    fn granted(&self, asker: &Asker, tool: &str) -> bool {
        let Some(conversation) = asker.conversation else {
            return false;
        };
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(asker.person.clone(), conversation, tool.to_string()))
    }

    /// Do what `decision` says with a held call, and log it. "Allow for
    /// this conversation" lets the same tool run without asking for the
    /// rest of `conversation`, or of the one its card is in.
    ///
    /// Not said on the stream: the caller says so with [`changed`] once
    /// the card in the chat is put back, so a page that refetches on
    /// hearing it finds the answered card and not a pending one.
    ///
    /// [`changed`]: Approvals::changed
    pub(crate) async fn settle(
        &self,
        held: Held,
        decision: Decision,
        via: Via,
        conversation: Option<i64>,
    ) -> (ApprovalCard, String) {
        if decision == Decision::Conversation
            && let Some(conversation) = conversation.or(held.asker.conversation)
        {
            self.grant(&held.asker.person, conversation, &held.name);
        }
        let mut logged = entry(&held, &held.card);
        let (card, reply) = match decision {
            Decision::Deny => held.deny(),
            Decision::Once | Decision::Conversation => held.run().await,
        };
        logged.status = status_name(card.status).into();
        logged.outcome = card.outcome.clone();
        logged.decision = Some(decision_name(decision).into());
        logged.decided_via = Some(via.as_str().into());
        logged.decided_at = Some(Utc::now());
        self.keep(&logged).await;
        (card, reply)
    }

    /// Tell `person`'s own connections what their card now says.
    pub(crate) fn changed(&self, person: &str, card: &ApprovalCard) {
        self.bus.publish(Event::ApprovalChanged {
            person: person.to_string(),
            id: card.id.clone(),
            status: status_name(card.status).into(),
        });
    }

    async fn keep(&self, entry: &ApprovalEntry) {
        if let Some(ledger) = &self.ledger
            && let Err(e) = ledger.settled(entry).await
        {
            tracing::warn!("[approvals] what was decided was not logged: {e}");
        }
    }

    /// Let go of everything that lapsed: logged as lapsed, and said, so
    /// the panel's count goes down without anybody asking it.
    pub(crate) async fn sweep(&self) {
        let now = Instant::now();
        let lapsed: Vec<Held> = {
            let mut held = self.lock();
            let ids: Vec<String> = held
                .iter()
                .filter(|(_, h)| h.lapses <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| held.remove(id)).collect()
        };
        for held in lapsed {
            let mut card = held.card.clone();
            card.status = ApprovalStatus::Lapsed;
            let mut logged = entry(&held, &card);
            logged.decided_at = Some(held.expires_at);
            tracing::info!(
                "[approvals] {} lapsed, unanswered by {}",
                held.name,
                held.asker.person
            );
            self.keep(&logged).await;
            self.changed(&held.asker.person, &card);
        }
    }

    pub(crate) fn spawn_sweep(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut every = tokio::time::interval(SWEEP_EVERY);
            loop {
                every.tick().await;
                self.sweep().await;
            }
        })
    }

    /// A page of `person`'s history, newest first.
    pub(crate) async fn history(
        &self,
        person: &str,
        before: Option<i64>,
    ) -> Result<History, String> {
        let Some(ledger) = &self.ledger else {
            return Ok(History {
                entries: Vec::new(),
                next: None,
                kept: false,
            });
        };
        let mut rows = ledger.page(person, before, PAGE as i64 + 1).await?;
        let more = rows.len() > PAGE;
        rows.truncate(PAGE);
        let next = more.then(|| rows.last().map(|r| r.id)).flatten();
        Ok(History {
            entries: rows.into_iter().map(decided).collect(),
            next,
            kept: true,
        })
    }
}

fn decided(row: ApprovalLogged) -> Decided {
    let e = row.entry;
    Decided {
        id: row.id,
        approval: e.approval,
        tool: e.tool,
        icon: e.icon,
        title: e.title,
        summary: e.summary,
        status: e.status,
        decision: e.decision,
        outcome: e.outcome,
        decided_via: e.decided_via,
        asked_via: e.asked_via,
        asked_at: e.asked_at,
        decided_at: e.decided_at,
        conversation_id: e.conversation_id,
    }
}

/// Where a held call waits for its answer.
fn asked_via(asker: &Asker) -> Via {
    if asker.satellite.is_some() {
        Via::Voice
    } else {
        Via::App
    }
}

/// The log's row for a held call, its card as `card` says now: not yet
/// decided, which the caller fills in when it is.
fn entry(held: &Held, card: &ApprovalCard) -> ApprovalEntry {
    ApprovalEntry {
        approval: card.id.clone(),
        person: held.asker.person.clone(),
        tool: held.label.clone(),
        icon: card.icon.clone(),
        title: card.title.clone(),
        summary: summary(card),
        status: status_name(card.status).into(),
        decision: None,
        outcome: card.outcome.clone(),
        asked_via: asked_via(&held.asker).as_str().into(),
        decided_via: None,
        asked_at: held.asked_at,
        decided_at: None,
        conversation_id: held.asker.conversation,
    }
}

fn status_name(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Allowed => "allowed",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Lapsed => "lapsed",
        ApprovalStatus::Failed => "failed",
    }
}

fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::Once => "once",
        Decision::Conversation => "conversation",
        Decision::Deny => "deny",
    }
}

/// What a card would do, in a line: its fields, each as the card labels
/// it, a value that reads like a key or a password left out. Never a
/// mail's body, which is the mail.
pub(crate) fn summary(card: &ApprovalCard) -> String {
    let line = card
        .fields
        .iter()
        .map(|f| {
            if looks_secret(&f.label) {
                format!("{}: ••••", f.label)
            } else {
                format!(
                    "{}: {}",
                    f.label,
                    f.value.split_whitespace().collect::<Vec<_>>().join(" ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let line = if line.is_empty() {
        card.subtitle.clone()
    } else {
        line
    };
    if line.chars().count() <= SUMMARY_CHARS {
        return line;
    }
    let cut: String = line.chars().take(SUMMARY_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// A field a remote tool might be handed a credential in. Shown on the
/// card, which is the person's own screen; never kept in the log.
fn looks_secret(label: &str) -> bool {
    let label = label.to_lowercase().replace(['_', '-'], " ");
    [
        "password",
        "passphrase",
        "secret",
        "token",
        "api key",
        "apikey",
        "private key",
        "authorization",
        "credential",
    ]
    .iter()
    .any(|word| label.contains(word))
}

impl Held {
    pub(crate) fn tool_name(&self) -> &str {
        &self.name
    }

    /// Run what was held, with what was held. Answers the card as it now
    /// stands and a sentence for the person.
    async fn run(mut self) -> (ApprovalCard, String) {
        match self.tool.execute(self.args).await {
            Ok(result) => {
                self.card.status = ApprovalStatus::Allowed;
                self.card.outcome = Some("Done".into());
                let said = gist(&result);
                let reply = if said.is_empty() {
                    "Done.".to_string()
                } else {
                    format!("Done. {said}")
                };
                (self.card, reply)
            }
            Err(e) => {
                self.card.status = ApprovalStatus::Failed;
                self.card.outcome = Some(e.to_string());
                (self.card, format!("That didn't work: {e}"))
            }
        }
    }

    fn deny(mut self) -> (ApprovalCard, String) {
        self.card.status = ApprovalStatus::Denied;
        self.card.outcome = Some("Denied".into());
        (self.card, "All right, I didn't.".to_string())
    }
}

/// The first line of a result, short enough to say: what the model
/// needs to know on the next turn, like an issue's id.
fn gist(result: &Value) -> String {
    let text = match result {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut out: String = line.chars().take(200).collect();
    if line.chars().count() > 200 {
        out.push('…');
    }
    out
}

fn new_id() -> String {
    format!("{:024x}", rand::random::<u128>() >> 32)
}

/// How a remote tool's call is drawn on its card.
#[derive(Clone)]
pub(crate) struct ToolCard {
    /// "Let Linear create issue?"
    pub(crate) title: String,
    /// "Linear"
    pub(crate) subtitle: String,
    pub(crate) icon: Option<String>,
    /// The tool's input schema, whose properties label the arguments.
    pub(crate) schema: Value,
}

impl ToolCard {
    fn for_args(&self, args: &Value) -> ApprovalCard {
        let properties = self.schema.get("properties");
        let fields = args
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, v)| !v.is_null())
            .map(|(key, value)| CardField {
                label: properties
                    .and_then(|p| p.get(key))
                    .and_then(|p| p.get("title"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| humanise(key)),
                value: shown(value),
            })
            .collect();
        ApprovalCard {
            id: String::new(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            icon: self.icon.clone(),
            fields,
            body: None,
            allow_conversation: true,
            status: ApprovalStatus::Pending,
            outcome: None,
        }
    }
}

/// `team_id` → "Team id".
pub(crate) fn humanise(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn shown(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Array(items) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    if text.chars().count() <= MAX_FIELD {
        return text;
    }
    let cut: String = text.chars().take(MAX_FIELD).collect();
    format!("{cut}…")
}

/// A tool set to Ask first: the model sees it as it is, and calling it
/// holds the call for the person instead of running it.
pub(crate) struct AskFirst {
    inner: Arc<dyn Tool>,
    approvals: Arc<Approvals>,
    card: ToolCard,
}

impl AskFirst {
    pub(crate) fn new(inner: Arc<dyn Tool>, approvals: Arc<Approvals>, card: ToolCard) -> Self {
        Self {
            inner,
            approvals,
            card,
        }
    }
}

#[async_trait]
impl Tool for AskFirst {
    fn descriptor(&self) -> ToolDescriptor {
        self.inner.descriptor()
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        let name = self.inner.descriptor().name;
        let Some(asker) = asker() else {
            return Err(niles_tools::Error::Remote(format!(
                "{name} waits for a person's yes, and nobody is here to give it"
            )));
        };
        if self.approvals.granted(&asker, &name) {
            return self.inner.execute(args).await;
        }
        let card = self.approvals.hold(
            &asker,
            self.inner.clone(),
            &self.card.subtitle,
            args.clone(),
            self.card.for_args(&args),
        );
        tracing::info!("[approvals] {name} is waiting for {}'s yes", asker.person);
        let note = if asker.satellite.is_some() {
            "Nothing has happened yet. Ask the person in one short sentence, ending with a question mark, whether to go ahead. Niles hears their yes or no itself."
        } else {
            "Nothing has happened yet. Under your reply the person has a card with Allow and Deny. Say in one short sentence what is waiting for them. Do not ask them to type yes."
        };
        Ok(json!({
            "waiting_for_approval": true,
            "shown_to_the_person": card.title,
            "note": note,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Echo(&'static str);

    #[async_trait]
    impl Tool for Echo {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: self.0.into(),
                description: "Echo.".into(),
                parameters: json!({ "type": "object" }),
            }
        }
        async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
            Ok(json!({ "made": args["title"] }))
        }
    }

    /// Counts its runs, so a test can tell a deny from an allow.
    #[derive(Default)]
    struct Counted(AtomicUsize);

    #[async_trait]
    impl Tool for Counted {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "linear__create_issue".into(),
                description: "Counted.".into(),
                parameters: json!({ "type": "object" }),
            }
        }
        async fn execute(&self, _args: Value) -> niles_tools::Result<Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!("LIN-1"))
        }
    }

    /// The log as a list, paged the way the table is.
    #[derive(Default)]
    struct Kept(Mutex<Vec<ApprovalLogged>>);

    impl Kept {
        fn of(&self, approval: &str) -> Option<ApprovalEntry> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.entry.approval == approval)
                .map(|r| r.entry.clone())
        }
    }

    #[async_trait]
    impl Ledger for Kept {
        async fn asked(&self, entry: &ApprovalEntry) -> Result<(), String> {
            let mut rows = self.0.lock().unwrap();
            if !rows.iter().any(|r| r.entry.approval == entry.approval) {
                let id = rows.len() as i64 + 1;
                rows.push(ApprovalLogged {
                    id,
                    entry: entry.clone(),
                });
            }
            Ok(())
        }
        async fn settled(&self, entry: &ApprovalEntry) -> Result<(), String> {
            let mut rows = self.0.lock().unwrap();
            match rows.iter_mut().find(|r| r.entry.approval == entry.approval) {
                Some(row) => row.entry = entry.clone(),
                None => {
                    let id = rows.len() as i64 + 1;
                    rows.push(ApprovalLogged {
                        id,
                        entry: entry.clone(),
                    });
                }
            }
            Ok(())
        }
        async fn page(
            &self,
            person: &str,
            before: Option<i64>,
            limit: i64,
        ) -> Result<Vec<ApprovalLogged>, String> {
            let mut rows: Vec<ApprovalLogged> = self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.entry.person == person && r.entry.status != "pending")
                .filter(|r| before.is_none_or(|b| r.id < b))
                .cloned()
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.id));
            rows.truncate(limit as usize);
            Ok(rows)
        }
    }

    fn card() -> ToolCard {
        ToolCard {
            title: "Let Linear create issue?".into(),
            subtitle: "Linear".into(),
            icon: Some("linear".into()),
            schema: json!({ "properties": { "title": { "title": "Title" } } }),
        }
    }

    fn in_the_app(conversation: Option<i64>) -> Asker {
        Asker {
            person: "person:mark".into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation,
        }
    }

    fn ask_first(approvals: &Arc<Approvals>) -> AskFirst {
        AskFirst::new(
            Arc::new(Echo("linear__create_issue")),
            approvals.clone(),
            card(),
        )
    }

    fn logged() -> (Arc<Approvals>, Arc<Kept>, EventBus) {
        let kept = Arc::new(Kept::default());
        let bus = EventBus::default();
        let approvals = Arc::new(Approvals::new(bus.clone(), Some(kept.clone())));
        (approvals, kept, bus)
    }

    /// One held call of `who`'s, on a tool that counts its runs.
    fn held_for(
        approvals: &Approvals,
        who: &str,
        conversation: Option<i64>,
    ) -> (String, Arc<Counted>) {
        let tool = Arc::new(Counted::default());
        let asker = Asker {
            person: who.into(),
            ..in_the_app(conversation)
        };
        let card = approvals.hold(
            &asker,
            tool.clone(),
            "Linear",
            json!({ "title": "x" }),
            card().for_args(&json!({ "title": "x" })),
        );
        (card.id, tool)
    }

    /// Let the spawned "asked" write land.
    async fn settle_writes() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn holds_the_call_instead_of_running_it() {
        let approvals = Arc::new(Approvals::default());
        let started = Instant::now();
        let said = ASKER
            .scope(
                in_the_app(None),
                ask_first(&approvals).execute(json!({ "title": "WLED presets", "team_id": "t1" })),
            )
            .await
            .unwrap();
        assert_eq!(said["waiting_for_approval"], true);
        let card = approvals.for_turn("person:mark", started).expect("a card");
        assert_eq!(card.status, ApprovalStatus::Pending);
        assert_eq!(
            card.fields,
            [
                CardField {
                    label: "Team id".into(),
                    value: "t1".into()
                },
                CardField {
                    label: "Title".into(),
                    value: "WLED presets".into()
                },
            ]
        );
        assert!(card.allow_conversation);
    }

    #[tokio::test]
    async fn runs_exactly_what_was_held_and_only_for_its_person() {
        let approvals = Arc::new(Approvals::default());
        let started = Instant::now();
        ASKER
            .scope(
                in_the_app(None),
                ask_first(&approvals).execute(json!({ "title": "WLED presets" })),
            )
            .await
            .unwrap();
        let id = approvals.for_turn("person:mark", started).unwrap().id;
        assert!(
            approvals.take(&id, "person:majse").is_none(),
            "not hers to answer"
        );
        let held = approvals.take(&id, "person:mark").unwrap();
        let (card, reply) = approvals.settle(held, Decision::Once, Via::App, None).await;
        assert_eq!(card.status, ApprovalStatus::Allowed);
        assert_eq!(reply, "Done. {\"made\":\"WLED presets\"}");
        assert!(
            approvals.take(&id, "person:mark").is_none(),
            "answered once"
        );
    }

    #[tokio::test]
    async fn allowed_for_the_conversation_runs_without_asking_again() {
        let approvals = Arc::new(Approvals::default());
        approvals.grant("person:mark", 7, "linear__create_issue");
        let result = ASKER
            .scope(
                in_the_app(Some(7)),
                ask_first(&approvals).execute(json!({ "title": "x" })),
            )
            .await
            .unwrap();
        assert_eq!(result, json!({ "made": "x" }));
        // Another conversation still asks.
        let held = ASKER
            .scope(
                in_the_app(Some(8)),
                ask_first(&approvals).execute(json!({ "title": "x" })),
            )
            .await
            .unwrap();
        assert_eq!(held["waiting_for_approval"], true);
    }

    #[tokio::test]
    async fn a_question_asked_aloud_is_answered_at_its_satellite() {
        let approvals = Arc::new(Approvals::default());
        let kitchen: SocketAddr = "192.168.1.20:40000".parse().unwrap();
        let spoken = Asker {
            satellite: Some(kitchen),
            ..in_the_app(None)
        };
        ASKER
            .scope(
                spoken,
                ask_first(&approvals).execute(json!({ "title": "x" })),
            )
            .await
            .unwrap();
        let (id, asker) = approvals.spoken(kitchen).expect("waiting");
        assert_eq!(asker.slug.as_deref(), Some("mark"));
        assert!(
            approvals
                .spoken("192.168.1.21:40000".parse().unwrap())
                .is_none()
        );
        // Spoken, it is never "for this conversation", and it is not a card.
        assert!(
            approvals
                .for_turn("person:mark", Instant::now() - Duration::from_secs(60))
                .is_none()
        );
        // It is in the panel, though, to be answered from the phone.
        let pending = approvals.pending("person:mark");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].via, "voice");
        assert!(!pending[0].card.allow_conversation);
        let held = approvals.take(&id, "person:mark").unwrap();
        let (card, _) = approvals
            .settle(held, Decision::Deny, Via::Voice, None)
            .await;
        assert_eq!(card.status, ApprovalStatus::Denied);
    }

    #[tokio::test]
    async fn nobody_to_ask_is_an_error_not_a_hold() {
        let approvals = Arc::new(Approvals::default());
        let err = ask_first(&approvals).execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("nobody is here"));
    }

    #[test]
    fn humanises_a_key() {
        assert_eq!(humanise("team_id"), "Team id");
        assert_eq!(humanise("dueDate"), "DueDate");
    }

    #[tokio::test]
    async fn the_panel_lists_only_the_callers_own() {
        let approvals = Approvals::default();
        let (mine, _) = held_for(&approvals, "person:mark", Some(3));
        held_for(&approvals, "person:majse", None);
        let pending = approvals.pending("person:mark");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].card.id, mine);
        assert_eq!(pending[0].tool, "Linear");
        assert_eq!(pending[0].summary, "Title: x");
        assert_eq!(pending[0].conversation_id, Some(3));
        assert_eq!(pending[0].via, "app");
        assert!(pending[0].expires_at > pending[0].asked_at);
        assert!(approvals.pending("email:anyone").is_empty());
    }

    #[tokio::test]
    async fn by_its_id_only_its_person_finds_it() {
        let approvals = Approvals::default();
        let (id, _) = held_for(&approvals, "person:mark", Some(3));
        assert_eq!(
            approvals.placed(&id, "person:majse"),
            Err(Undecidable::NoSuchApproval),
            "somebody else's is not there at all"
        );
        assert_eq!(
            approvals.placed(&id, "person:mark"),
            Ok(Placed {
                conversation: Some(3),
                allow_conversation: true
            })
        );
        approvals.lock().get_mut(&id).unwrap().lapses = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            approvals.placed(&id, "person:mark"),
            Err(Undecidable::NotWaiting)
        );
    }

    #[tokio::test]
    async fn allow_once_runs_and_grants_nothing() {
        let approvals = Approvals::default();
        let (id, tool) = held_for(&approvals, "person:mark", Some(3));
        let held = approvals.take(&id, "person:mark").unwrap();
        let (card, _) = approvals
            .settle(held, Decision::Once, Via::App, Some(3))
            .await;
        assert_eq!(card.status, ApprovalStatus::Allowed);
        assert_eq!(tool.0.load(Ordering::SeqCst), 1);
        assert!(!approvals.granted(&in_the_app(Some(3)), "linear__create_issue"));
    }

    #[tokio::test]
    async fn allow_for_the_conversation_runs_and_grants_it() {
        let approvals = Approvals::default();
        let (id, tool) = held_for(&approvals, "person:mark", Some(3));
        let held = approvals.take(&id, "person:mark").unwrap();
        // From the panel, where the conversation is the card's own.
        approvals
            .settle(held, Decision::Conversation, Via::App, None)
            .await;
        assert_eq!(tool.0.load(Ordering::SeqCst), 1);
        assert!(approvals.granted(&in_the_app(Some(3)), "linear__create_issue"));
        assert!(!approvals.granted(&in_the_app(Some(4)), "linear__create_issue"));
    }

    #[tokio::test]
    async fn deny_runs_nothing() {
        let approvals = Approvals::default();
        let (id, tool) = held_for(&approvals, "person:mark", Some(3));
        let held = approvals.take(&id, "person:mark").unwrap();
        let (card, reply) = approvals
            .settle(held, Decision::Deny, Via::App, Some(3))
            .await;
        assert_eq!(card.status, ApprovalStatus::Denied);
        assert_eq!(reply, "All right, I didn't.");
        assert_eq!(tool.0.load(Ordering::SeqCst), 0);
        assert!(!approvals.granted(&in_the_app(Some(3)), "linear__create_issue"));
    }

    #[tokio::test]
    async fn every_outcome_is_logged_as_what_became_of_it() {
        let (approvals, kept, _) = logged();
        let mut ids = Vec::new();
        for decision in [Decision::Once, Decision::Conversation, Decision::Deny] {
            let (id, _) = held_for(&approvals, "person:mark", Some(3));
            settle_writes().await;
            assert_eq!(kept.of(&id).unwrap().status, "pending", "logged when asked");
            let held = approvals.take(&id, "person:mark").unwrap();
            approvals.settle(held, decision, Via::App, Some(3)).await;
            ids.push(id);
        }
        let once = kept.of(&ids[0]).unwrap();
        assert_eq!(
            (
                once.status.as_str(),
                once.decision.as_deref(),
                once.decided_via.as_deref()
            ),
            ("allowed", Some("once"), Some("app"))
        );
        assert_eq!(once.outcome.as_deref(), Some("Done"));
        assert_eq!(once.tool, "Linear");
        assert_eq!(once.title, "Let Linear create issue?");
        assert_eq!(once.conversation_id, Some(3));
        assert!(once.decided_at.is_some());
        assert_eq!(
            kept.of(&ids[1]).unwrap().decision.as_deref(),
            Some("conversation")
        );
        let denied = kept.of(&ids[2]).unwrap();
        assert_eq!(
            (denied.status.as_str(), denied.decision.as_deref()),
            ("denied", Some("deny"))
        );
    }

    #[tokio::test]
    async fn a_lapse_is_logged_and_said() {
        let (approvals, kept, bus) = logged();
        let (id, tool) = held_for(&approvals, "person:mark", None);
        settle_writes().await;
        let mut heard = bus.subscribe();
        approvals.lock().get_mut(&id).unwrap().lapses = Instant::now() - Duration::from_secs(1);
        approvals.sweep().await;
        let lapsed = kept.of(&id).unwrap();
        assert_eq!(lapsed.status, "lapsed");
        assert_eq!(lapsed.decision, None);
        assert_eq!(lapsed.decided_via, None);
        assert!(lapsed.decided_at.is_some());
        assert_eq!(tool.0.load(Ordering::SeqCst), 0);
        assert!(approvals.pending("person:mark").is_empty());
        match heard.recv().await.unwrap() {
            Event::ApprovalChanged {
                person,
                id: said,
                status,
            } => {
                assert_eq!(
                    (person.as_str(), said.as_str(), status.as_str()),
                    ("person:mark", id.as_str(), "lapsed")
                );
            }
            other => panic!("expected ApprovalChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn asking_is_said_to_the_person_it_waits_for() {
        let (approvals, _, bus) = logged();
        let mut heard = bus.subscribe();
        let (id, _) = held_for(&approvals, "person:majse", None);
        match heard.recv().await.unwrap() {
            Event::ApprovalChanged {
                person,
                id: said,
                status,
            } => {
                assert_eq!(person, "person:majse");
                assert_eq!(said, id);
                assert_eq!(status, "pending");
            }
            other => panic!("expected ApprovalChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_is_the_persons_own_a_page_at_a_time() {
        let (approvals, _, _) = logged();
        for _ in 0..(PAGE + 5) {
            let (id, _) = held_for(&approvals, "person:mark", None);
            settle_writes().await;
            let held = approvals.take(&id, "person:mark").unwrap();
            approvals.settle(held, Decision::Deny, Via::App, None).await;
        }
        let (theirs, _) = held_for(&approvals, "person:majse", None);
        settle_writes().await;
        let held = approvals.take(&theirs, "person:majse").unwrap();
        approvals.settle(held, Decision::Deny, Via::App, None).await;
        // And one still waiting, which is not history yet.
        held_for(&approvals, "person:mark", None);
        settle_writes().await;

        let first = approvals.history("person:mark", None).await.unwrap();
        assert!(first.kept);
        assert_eq!(first.entries.len(), PAGE);
        assert!(
            first.entries.windows(2).all(|w| w[0].id > w[1].id),
            "newest first"
        );
        assert!(first.entries.iter().all(|e| e.approval != theirs));
        let next = first.next.expect("a second page");
        let second = approvals.history("person:mark", Some(next)).await.unwrap();
        assert_eq!(second.entries.len(), 5);
        assert_eq!(second.next, None);
        assert!(second.entries.iter().all(|e| e.id < next));
    }

    #[tokio::test]
    async fn without_a_database_history_is_empty_and_says_so() {
        let history = Approvals::default()
            .history("person:mark", None)
            .await
            .unwrap();
        assert!(history.entries.is_empty());
        assert!(!history.kept);
    }

    #[tokio::test]
    async fn the_log_keeps_a_line_not_the_arguments() {
        let (approvals, kept, _) = logged();
        let long = "word ".repeat(400);
        let args = json!({ "title": long, "api_key": "sk-live-123", "auth_token": "tok-9" });
        let card = approvals.hold(
            &in_the_app(None),
            Arc::new(Counted::default()),
            "Linear",
            args.clone(),
            card().for_args(&args),
        );
        settle_writes().await;
        let entry = kept.of(&card.id).unwrap();
        assert!(!entry.summary.contains("sk-live-123"), "{}", entry.summary);
        assert!(!entry.summary.contains("tok-9"), "{}", entry.summary);
        assert!(entry.summary.contains("Api key: ••••"), "{}", entry.summary);
        assert!(entry.summary.chars().count() <= SUMMARY_CHARS + 1);
        assert!(entry.summary.ends_with('…'));
    }

    #[test]
    fn a_mails_summary_is_who_and_what_about_never_the_mail() {
        let mail = ApprovalCard {
            id: "m".into(),
            title: "Send this email?".into(),
            subtitle: "From your Gmail, mark@gmail.com".into(),
            icon: Some("mail".into()),
            fields: vec![
                CardField {
                    label: "To".into(),
                    value: "jakob@example.com".into(),
                },
                CardField {
                    label: "Subject".into(),
                    value: "Running late".into(),
                },
            ],
            body: Some("Hi Jakob, the door code is 4321.".into()),
            allow_conversation: false,
            status: ApprovalStatus::Pending,
            outcome: None,
        };
        let line = summary(&mail);
        assert_eq!(line, "To: jakob@example.com · Subject: Running late");
        assert!(!line.contains("4321"));
    }
}

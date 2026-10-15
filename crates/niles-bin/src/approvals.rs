//! Approvals: what Niles holds for a person's yes instead of doing on
//! the model's word. See "Approvals" in ARCHITECTURE.md.
//!
//! A held call keeps the tool and its arguments exactly as the model
//! sent them. The person decides — a button on the card in the app, or
//! a spoken yes at the satellite that asked — and Niles runs what it
//! held, never what the model says next.
//!
//! Held calls are kept in memory. A restart lapses them; a card still
//! pending in a conversation then reads as lapsed.

use async_trait::async_trait;
use niles_api::chat::{ApprovalCard, ApprovalStatus, CardField};
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

pub(crate) struct Held {
    asker: Asker,
    tool: Arc<dyn Tool>,
    name: String,
    args: Value,
    pub(crate) card: ApprovalCard,
    made: Instant,
    lapses: Instant,
}

#[derive(Default)]
pub(crate) struct Approvals {
    held: Mutex<HashMap<String, Held>>,
    /// "Allow for this conversation": (person, conversation, tool).
    grants: Mutex<HashSet<(String, i64, String)>>,
}

impl Approvals {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keep `tool` called with `args` for `asker`'s yes. Answers the
    /// card as it is to be shown.
    pub(crate) fn hold(
        &self,
        asker: &Asker,
        tool: Arc<dyn Tool>,
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
        let name = tool.descriptor().name;
        let mut held = self.lock();
        held.retain(|_, h| h.lapses > now);
        held.insert(
            card.id.clone(),
            Held {
                asker: asker.clone(),
                tool,
                name,
                args,
                card: card.clone(),
                made: now,
                lapses: now + wait,
            },
        );
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
}

impl Held {
    pub(crate) fn tool_name(&self) -> &str {
        &self.name
    }

    /// Run what was held, with what was held. Answers the card as it now
    /// stands and a sentence for the person.
    pub(crate) async fn run(mut self) -> (ApprovalCard, String) {
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

    pub(crate) fn deny(mut self) -> (ApprovalCard, String) {
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
        let (card, reply) = approvals.take(&id, "person:mark").unwrap().run().await;
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
        let (card, _) = approvals.take(&id, "person:mark").unwrap().deny();
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
}

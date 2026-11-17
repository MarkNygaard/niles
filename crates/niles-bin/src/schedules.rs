//! Each person's schedules, run on time: the assistant answers the
//! instruction as that person, into a new conversation in their chat,
//! and they are told (NIL-81).
//!
//! A run is a typed chat turn, through the same `AppChat::reply` a
//! message takes, so the person's own tools are offered and a tool that
//! asks puts its card in that conversation. It is never a satellite's
//! turn and never spoken: the turn's asker has no satellite.

use crate::app::AppChat;
use crate::approvals::{Approvals, AskFirst, ToolCard};
use crate::notify::Notify;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use niles_api::chat::Chat;
use niles_config::ConfigStore;
use niles_schedules::{NewSchedule, Outcome, Run, Schedule, ScheduleStore, When};
use niles_tools::{DynamicTools, Tool, ToolDescriptor};
use niles_webpush::Notice;
use niles_webpush::inbox::{Kind, NOTIFICATIONS_URL};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// How often the clock is read for what is due.
const TICK: Duration = Duration::from_secs(30);
/// Later than this past its time, a run was missed rather than late:
/// Niles was down, and it isn't caught up.
const LATE: chrono::TimeDelta = chrono::TimeDelta::minutes(2);
/// How much of the answer the notification shows.
const GLIMPSE: usize = 160;

const NO_ASSISTANT: &str =
    "Schedules run on Claude or ChatGPT, and neither is switched on under Settings.";

/// What became of the instruction.
pub(crate) struct Answered {
    pub(crate) reply: String,
    pub(crate) conversation: Option<i64>,
    /// A tool's card waits in that conversation.
    pub(crate) waiting: bool,
    /// Why the assistant didn't answer, when Niles's own model did.
    pub(crate) fallback: Option<String>,
}

/// Who answers a run: the app's chat, or a test's.
#[async_trait]
pub(crate) trait Answer: Send + Sync {
    /// Whether an assistant is switched on to answer.
    fn assistant(&self) -> bool;
    async fn answer(&self, person: &str, said: &str) -> Result<Answered, String>;
}

/// The app's chat answers, as the person, in a new conversation.
pub(crate) struct ChatAnswer {
    pub(crate) chat: Arc<AppChat>,
    pub(crate) settings: Arc<ConfigStore>,
}

#[async_trait]
impl Answer for ChatAnswer {
    fn assistant(&self) -> bool {
        self.chat
            .ctx
            .get()
            .is_some_and(crate::dispatch::assistant_on)
    }

    async fn answer(&self, person: &str, said: &str) -> Result<Answered, String> {
        let config = self.settings.current();
        // The chat is keyed by the person's address and voice, as when
        // they type; someone taken off People has no chat to run in.
        let (email, speaker) = config
            .auth
            .allowed
            .iter()
            .find(|p| niles_training::person_key(p.speaker.as_deref(), &p.email) == person)
            .map(|p| (p.email.clone(), p.speaker.clone()))
            .ok_or_else(|| "you are no longer on Settings → People".to_string())?;
        let reply = self
            .chat
            .reply(
                &email,
                speaker.as_deref(),
                said,
                Vec::new(),
                niles_api::chat::Where::New,
            )
            .await?;
        let waiting = match (reply.conversation_id, self.chat.ctx.get()) {
            (Some(id), Some(ctx)) => ctx
                .approvals
                .pending(person)
                .iter()
                .any(|w| w.conversation_id == Some(id)),
            _ => false,
        };
        Ok(Answered {
            reply: reply.reply,
            conversation: reply.conversation_id,
            waiting,
            fallback: reply.fallback,
        })
    }
}

/// A schedule whose time has come.
pub(crate) struct Due {
    person: String,
    schedule: Schedule,
    at: DateTime<Utc>,
}

/// How a run ended, as kept and as told.
struct Ended {
    outcome: Outcome,
    conversation: Option<i64>,
    reason: Option<String>,
    told: String,
}

pub(crate) struct Schedules {
    store: Arc<dyn ScheduleStore>,
    answer: Arc<dyn Answer>,
    notify: Arc<Notify>,
    settings: Arc<ConfigStore>,
}

impl Schedules {
    pub(crate) fn new(
        store: Arc<dyn ScheduleStore>,
        answer: Arc<dyn Answer>,
        notify: Arc<Notify>,
        settings: Arc<ConfigStore>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            answer,
            notify,
            settings,
        })
    }

    /// Reads the clock every [`TICK`] and runs what is due, each in its
    /// own task so a slow answer holds up nobody else's.
    pub(crate) fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this
                .store
                .abandon_running("Niles restarted while this ran.")
                .await
            {
                tracing::warn!("[schedules] {e}");
            }
            let mut clock = tokio::time::interval(TICK);
            clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                clock.tick().await;
                for due in this.take_due(Utc::now()).await {
                    let this = this.clone();
                    tokio::spawn(async move { this.run(&due.person, &due.schedule, due.at).await });
                }
            }
        })
    }

    fn tz(&self) -> Tz {
        self.settings
            .current()
            .home
            .timezone
            .parse()
            .unwrap_or(chrono_tz::UTC)
    }

    fn next(&self, schedule: &NewSchedule) -> Option<DateTime<Utc>> {
        schedule.when.next_after(Utc::now(), self.tz())
    }

    /// What is due by `now`, each moved on to its next time first, so
    /// nothing runs twice. One missed by more than [`LATE`] is noted as
    /// missed and not run.
    async fn take_due(&self, now: DateTime<Utc>) -> Vec<Due> {
        let due = match self.store.due(now).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!("[schedules] {e}");
                return Vec::new();
            }
        };
        let tz = self.tz();
        let mut runs = Vec::new();
        for (person, schedule) in due {
            let Some(at) = schedule.next_due else {
                continue;
            };
            if let Err(e) = self
                .store
                .set_next(schedule.id, schedule.when.next_after(now, tz))
                .await
            {
                tracing::warn!("[schedules] {} not moved on, so not run: {e}", schedule.id);
                continue;
            }
            if at < now - LATE {
                tracing::info!("[schedules] {person}'s {} was missed", schedule.id);
                if let Err(e) = self
                    .store
                    .start_run(&person, schedule.id, at, Outcome::Missed)
                    .await
                {
                    tracing::warn!("[schedules] {e}");
                }
                continue;
            }
            runs.push(Due {
                person,
                schedule,
                at,
            });
        }
        runs
    }

    /// One run, kept from start to end, and the person told.
    async fn run(&self, person: &str, schedule: &Schedule, due: DateTime<Utc>) {
        let run = match self
            .store
            .start_run(person, schedule.id, due, Outcome::Running)
            .await
        {
            Ok(Some(run)) => run,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("[schedules] {e}");
                return;
            }
        };
        tracing::info!("[schedules] running {person}'s {}", schedule.id);
        let ended = self.answered(person, schedule).await;
        if let Err(e) = self
            .store
            .finish_run(
                run,
                ended.outcome,
                ended.conversation,
                ended.reason.as_deref(),
            )
            .await
        {
            tracing::warn!("[schedules] {e}");
        }
        let notice = Notice {
            kind: Kind::Schedule,
            title: schedule.name.clone(),
            body: ended.told,
            url: ended.conversation.map_or_else(
                || NOTIFICATIONS_URL.to_string(),
                |id| format!("/#/chat?conversation={id}"),
            ),
            tag: format!("schedule-{}", schedule.id),
        };
        self.notify.person(person, &notice).await;
    }

    async fn answered(&self, person: &str, schedule: &Schedule) -> Ended {
        let failed = |conversation, why: String| Ended {
            outcome: Outcome::Failed,
            conversation,
            told: why.clone(),
            reason: Some(why),
        };
        if !self.answer.assistant() {
            return failed(None, NO_ASSISTANT.to_string());
        }
        // Named, so the conversation it starts is titled by it.
        let said = format!("{}: {}", schedule.name, schedule.instruction);
        match self.answer.answer(person, &said).await {
            Err(why) => failed(None, why),
            // Niles's own model has none of the person's tools; its
            // answer to an agent task is a guess, so the run failed.
            Ok(Answered {
                fallback: Some(why),
                conversation,
                ..
            }) => failed(conversation, format!("The assistant didn't answer: {why}")),
            Ok(answered) if answered.waiting => Ended {
                outcome: Outcome::Waiting,
                conversation: answered.conversation,
                reason: None,
                told: "Waiting for your yes on a card in the chat.".to_string(),
            },
            Ok(answered) => Ended {
                outcome: Outcome::Done,
                conversation: answered.conversation,
                reason: None,
                told: glimpse(&answered.reply),
            },
        }
    }

    pub(crate) async fn add(
        &self,
        person: &str,
        new: &NewSchedule,
    ) -> niles_schedules::Result<Schedule> {
        let kept = self.store.add(person, new, self.next(new)).await?;
        tracing::info!("[schedules] {person} added schedule {}", kept.id);
        Ok(kept)
    }

    /// Runs the person's schedule now, once, without moving its next
    /// time. False when it isn't theirs.
    pub(crate) async fn run_now(
        self: &Arc<Self>,
        person: &str,
        id: i64,
    ) -> niles_schedules::Result<bool> {
        let Some(schedule) = self.store.get(person, id).await? else {
            return Ok(false);
        };
        let this = self.clone();
        let person = person.to_string();
        tokio::spawn(async move { this.run(&person, &schedule, Utc::now()).await });
        Ok(true)
    }
}

/// The start of the answer, on a lock screen.
fn glimpse(reply: &str) -> String {
    let flat = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= GLIMPSE {
        return flat;
    }
    let cut: String = flat.chars().take(GLIMPSE - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The chat tool that keeps a schedule.
const MAKE: &str = "make_schedule";

/// The person typing in the app's chat, if this turn is that: a
/// schedule is someone's own, and a voice on a satellite has none.
fn typing_now() -> Option<String> {
    crate::approvals::asker()
        .filter(|a| a.in_chat && a.satellite.is_none())
        .map(|a| a.person)
        .filter(|p| p.starts_with("person:") || p.starts_with("email:"))
}

/// Keeps a schedule for the person typing, once they allow it on its
/// card.
struct MakeSchedule(Arc<Schedules>);

fn invalid(reason: impl Into<String>) -> niles_tools::Error {
    niles_tools::Error::InvalidArgs {
        tool: MAKE.into(),
        reason: reason.into(),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// The tool's flat arguments as a `When`. Flat, so its card reads as
/// a few labelled lines.
fn when_of(args: &Value) -> niles_tools::Result<When> {
    let at = || {
        str_arg(args, "at")
            .map(str::to_string)
            .ok_or_else(|| invalid("at is the time of day, HH:MM"))
    };
    match str_arg(args, "repeat").unwrap_or_default() {
        "daily" => Ok(When::Daily { at: at()? }),
        "weekdays" => Ok(When::Weekdays { at: at()? }),
        "days" => {
            let days = args
                .get("days")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("days names the days, e.g. [\"mon\"]"))?
                .iter()
                .map(|d| {
                    d.as_str()
                        .and_then(|d| d.parse::<chrono::Weekday>().ok())
                        .ok_or_else(|| invalid("a day is mon, tue, wed, thu, fri, sat or sun"))
                })
                .collect::<niles_tools::Result<Vec<_>>>()?;
            Ok(When::Days { days, at: at()? })
        }
        "every_hours" => {
            let hours = args
                .get("hours")
                .and_then(Value::as_u64)
                .and_then(|h| u8::try_from(h).ok())
                .ok_or_else(|| invalid("hours is 1, 2, 3, 4, 6, 8 or 12"))?;
            Ok(When::EveryHours { hours })
        }
        _ => Err(invalid("repeat is daily, weekdays, days or every_hours")),
    }
}

#[async_trait]
impl Tool for MakeSchedule {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: MAKE.into(),
            description: "Keep a schedule for the person typing: at the times given, \
                Niles's assistant answers the instruction as them, with their own tools, \
                in a new conversation in their chat, and notifies them. For a task the \
                assistant does again and again — 'every Monday at 8, my open Linear issues \
                by priority', 'each weekday at 7, what's in my calendar today'. Not for the \
                house's lights and devices, which are automations. At most once an hour. \
                The person allows it on a card first; nothing is kept until then."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "instruction", "repeat"],
                "properties": {
                    "name": { "type": "string", "title": "Name", "description": "A short name for it, 60 characters at most." },
                    "instruction": { "type": "string", "title": "Instruction", "description": "What to do each time, in the person's words, standing on its own: the run has no memory of this conversation." },
                    "repeat": { "type": "string", "enum": ["daily", "weekdays", "days", "every_hours"], "title": "Repeats" },
                    "at": { "type": "string", "title": "At", "description": "daily, weekdays and days: the time of day, HH:MM, 24-hour, in the household's time zone." },
                    "days": { "type": "array", "items": { "type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] }, "title": "Days", "description": "days only: which days." },
                    "hours": { "type": "integer", "enum": [1, 2, 3, 4, 6, 8, 12], "title": "Every (hours)", "description": "every_hours only: on the hour from midnight." }
                }
            }),
        }
    }

    fn offered(&self) -> bool {
        typing_now().is_some()
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        let person = typing_now().ok_or_else(|| {
            niles_tools::Error::Remote("a schedule is made by its person, in the app's chat".into())
        })?;
        let new = NewSchedule::new(
            str_arg(&args, "name").unwrap_or_default(),
            str_arg(&args, "instruction").unwrap_or_default(),
            when_of(&args)?,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let kept = self
            .0
            .add(&person, &new)
            .await
            .map_err(|e| niles_tools::Error::Remote(e.to_string()))?;
        let tz = self.0.tz();
        Ok(json!({
            "kept": kept.name,
            "next_run": kept.next_due.map(|at| at.with_timezone(&tz).format("%A %-d %B at %H:%M").to_string()),
            "note": "It is under My profile → Schedules, where it can be changed, paused or run now.",
        }))
    }
}

/// Offers `make_schedule` in the chat, behind its card.
pub(crate) fn offer(schedules: &Arc<Schedules>, dynamic: &DynamicTools, approvals: Arc<Approvals>) {
    let make = MakeSchedule(schedules.clone());
    let card = ToolCard {
        title: "Keep this schedule?".into(),
        subtitle: "Schedules".into(),
        icon: None,
        schema: make.descriptor().parameters,
    };
    let keywords = ["schedule", "every", "daily", "weekdays", "weekly", "hourly"]
        .map(str::to_string)
        .to_vec();
    dynamic.set(
        "schedules",
        keywords,
        vec![Arc::new(AskFirst::new(Arc::new(make), approvals, card)) as Arc<dyn Tool>],
    );
}

/// My profile → Schedules, for the API.
pub(crate) struct AppSchedules(pub(crate) Arc<Schedules>);

#[async_trait]
impl niles_api::schedules::PersonalSchedules for AppSchedules {
    fn assistant(&self) -> bool {
        self.0.answer.assistant()
    }

    async fn list(&self, person: &str) -> niles_schedules::Result<Vec<Schedule>> {
        self.0.store.list(person).await
    }

    async fn add(&self, person: &str, new: NewSchedule) -> niles_schedules::Result<Schedule> {
        self.0.add(person, &new).await
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        new: NewSchedule,
    ) -> niles_schedules::Result<Option<Schedule>> {
        self.0
            .store
            .update(person, id, &new, self.0.next(&new))
            .await
    }

    async fn remove(&self, person: &str, id: i64) -> niles_schedules::Result<bool> {
        self.0.store.remove(person, id).await
    }

    async fn pause(&self, person: &str, id: i64, paused: bool) -> niles_schedules::Result<bool> {
        let next = match self.0.store.get(person, id).await? {
            Some(schedule) if !paused => schedule.when.next_after(Utc::now(), self.0.tz()),
            Some(_) => None,
            None => return Ok(false),
        };
        self.0.store.set_paused(person, id, paused, next).await
    }

    async fn run_now(&self, person: &str, id: i64) -> niles_schedules::Result<bool> {
        self.0.run_now(person, id).await
    }

    async fn runs(&self, person: &str, id: i64) -> niles_schedules::Result<Option<Vec<Run>>> {
        self.0.store.runs(person, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_schedules::{MemorySchedules, When};
    use std::sync::Mutex;

    const MAJSE: &str = "person:majse";

    #[derive(Default)]
    struct Fake {
        off: bool,
        waiting: bool,
        fallback: Option<String>,
        said: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Answer for Fake {
        fn assistant(&self) -> bool {
            !self.off
        }

        async fn answer(&self, _person: &str, said: &str) -> Result<Answered, String> {
            self.said
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(said.to_string());
            Ok(Answered {
                reply: "Three issues are open; NIL-81 first.".into(),
                conversation: Some(42),
                waiting: self.waiting,
                fallback: self.fallback.clone(),
            })
        }
    }

    fn schedules(answer: Fake) -> (Arc<Schedules>, Arc<MemorySchedules>, Arc<Fake>) {
        let store = Arc::new(MemorySchedules::new());
        let answer = Arc::new(answer);
        let schedules = Schedules::new(
            store.clone(),
            answer.clone(),
            Arc::new(Notify::new(None, None, niles_core::EventBus::default())),
            Arc::new(ConfigStore::from_str_in_memory("").unwrap()),
        );
        (schedules, store, answer)
    }

    async fn kept(store: &MemorySchedules, due: DateTime<Utc>) -> Schedule {
        let new = NewSchedule::new(
            "Linear",
            "my open Linear issues by priority",
            When::EveryHours { hours: 1 },
        )
        .unwrap();
        store.add(MAJSE, &new, Some(due)).await.unwrap()
    }

    async fn run_due(schedules: &Schedules, now: DateTime<Utc>) {
        for due in schedules.take_due(now).await {
            schedules.run(&due.person, &due.schedule, due.at).await;
        }
    }

    #[tokio::test]
    async fn a_due_schedule_runs_once_into_a_conversation_and_moves_on() {
        let (schedules, store, answer) = schedules(Fake::default());
        let now = Utc::now();
        let schedule = kept(&store, now - chrono::TimeDelta::seconds(20)).await;
        run_due(&schedules, now).await;
        run_due(&schedules, now).await;
        assert_eq!(
            *answer.said.lock().unwrap(),
            ["Linear: my open Linear issues by priority"]
        );
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].outcome, Outcome::Done);
        assert_eq!(runs[0].conversation, Some(42));
        let next = store
            .get(MAJSE, schedule.id)
            .await
            .unwrap()
            .unwrap()
            .next_due;
        assert!(next.is_some_and(|at| at > now));
    }

    #[tokio::test]
    async fn a_missed_run_is_noted_and_not_caught_up() {
        let (schedules, store, answer) = schedules(Fake::default());
        let now = Utc::now();
        let schedule = kept(&store, now - chrono::TimeDelta::hours(3)).await;
        run_due(&schedules, now).await;
        assert!(answer.said.lock().unwrap().is_empty());
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs.len(), 1, "one miss, not one per hour missed");
        assert_eq!(runs[0].outcome, Outcome::Missed);
        let next = store
            .get(MAJSE, schedule.id)
            .await
            .unwrap()
            .unwrap()
            .next_due;
        assert!(next.is_some_and(|at| at > now), "the next one runs on time");
    }

    #[tokio::test]
    async fn a_run_held_on_a_card_is_waiting() {
        let (schedules, store, _) = schedules(Fake {
            waiting: true,
            ..Fake::default()
        });
        let now = Utc::now();
        let schedule = kept(&store, now).await;
        run_due(&schedules, now).await;
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs[0].outcome, Outcome::Waiting);
        assert_eq!(runs[0].conversation, Some(42));
    }

    #[tokio::test]
    async fn without_an_assistant_a_run_fails_saying_why() {
        let (schedules, store, answer) = schedules(Fake {
            off: true,
            ..Fake::default()
        });
        let now = Utc::now();
        let schedule = kept(&store, now).await;
        run_due(&schedules, now).await;
        assert!(answer.said.lock().unwrap().is_empty());
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs[0].outcome, Outcome::Failed);
        assert_eq!(runs[0].reason.as_deref(), Some(NO_ASSISTANT));
    }

    #[tokio::test]
    async fn an_answer_from_niles_own_model_is_a_failed_run() {
        let (schedules, store, _) = schedules(Fake {
            fallback: Some("the subscription has lapsed".into()),
            ..Fake::default()
        });
        let now = Utc::now();
        let schedule = kept(&store, now).await;
        run_due(&schedules, now).await;
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs[0].outcome, Outcome::Failed);
        assert_eq!(runs[0].conversation, Some(42), "the answer is still there");
    }

    #[tokio::test]
    async fn a_run_left_running_by_a_restart_is_failed() {
        let (_, store, _) = schedules(Fake::default());
        let schedule = kept(&store, Utc::now()).await;
        store
            .start_run(MAJSE, schedule.id, Utc::now(), Outcome::Running)
            .await
            .unwrap();
        store.abandon_running("restarted").await.unwrap();
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs[0].outcome, Outcome::Failed);
    }

    fn asker(satellite: Option<std::net::SocketAddr>) -> crate::approvals::Asker {
        crate::approvals::Asker {
            person: MAJSE.into(),
            slug: Some("majse".into()),
            satellite,
            conversation: None,
            in_chat: satellite.is_none(),
        }
    }

    #[tokio::test]
    async fn make_schedule_is_for_the_person_typing_never_a_voice() {
        let (schedules, _, _) = schedules(Fake::default());
        let make = MakeSchedule(schedules);
        assert!(!make.offered(), "nobody's turn");
        let voice = Some("10.0.0.9:10700".parse().unwrap());
        let on_a_satellite = crate::approvals::ASKER
            .scope(asker(voice), async { make.offered() })
            .await;
        assert!(!on_a_satellite);
        let typed = crate::approvals::ASKER
            .scope(asker(None), async { make.offered() })
            .await;
        assert!(typed);
    }

    #[tokio::test]
    async fn make_schedule_keeps_it_for_the_person_typing() {
        let (schedules, store, _) = schedules(Fake::default());
        let make = MakeSchedule(schedules);
        let said = crate::approvals::ASKER
            .scope(
                asker(None),
                make.execute(json!({
                    "name": "Linear on Monday",
                    "instruction": "my open Linear issues by priority",
                    "repeat": "days",
                    "days": ["mon"],
                    "at": "8:00",
                })),
            )
            .await
            .unwrap();
        assert_eq!(said["kept"], "Linear on Monday");
        let kept = store.list(MAJSE).await.unwrap();
        assert_eq!(
            kept[0].when,
            When::Days {
                days: vec![chrono::Weekday::Mon],
                at: "08:00".into()
            }
        );
        assert!(kept[0].next_due.is_some());
        assert!(store.list("person:mark").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn make_schedule_refuses_more_often_than_hourly() {
        let (schedules, store, _) = schedules(Fake::default());
        let make = MakeSchedule(schedules);
        let refused = crate::approvals::ASKER
            .scope(
                asker(None),
                make.execute(json!({
                    "name": "Mail",
                    "instruction": "anything new in my mail?",
                    "repeat": "every_hours",
                    "hours": 0,
                })),
            )
            .await;
        assert!(refused.is_err());
        assert!(store.list(MAJSE).await.unwrap().is_empty());
    }

    #[test]
    fn the_notification_shows_the_start_of_the_answer_on_one_line() {
        assert_eq!(
            glimpse("Three issues.\n\nNIL-81 first."),
            "Three issues. NIL-81 first."
        );
        let long = "word ".repeat(100);
        let shown = glimpse(&long);
        assert!(shown.chars().count() <= GLIMPSE);
        assert!(shown.ends_with('…'));
    }
}

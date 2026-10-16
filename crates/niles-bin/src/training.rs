//! The training tools: the asker's own workouts, as their Strava has
//! them.
//!
//! Whose comes from who is asking, never from an argument: the voice
//! [`profile::SPEAKER`] names, or in the app's chat the person signed
//! in ([`CHAT_PERSON`]), or else the turn's [`approvals::ASKER`]: what
//! Claude's and ChatGPT's calls carry, a background job's among them,
//! for somebody signed in without a voice. Somebody else's data cannot
//! be asked for by name, and an unrecognised voice has none.
//!
//! And their training plan's day, [`TrainingToday`]: what they are to
//! run, from the plan Claude or ChatGPT made them.

use crate::{approvals, profile};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use niles_artifacts::ArtifactStore;
use niles_artifacts::plan_day::{self, PlanDay};
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use niles_training::{Activity, Kind, TrainingStore, person_key};
use serde_json::{Value, json};
use std::sync::Arc;

tokio::task_local! {
    /// The chat's key for whoever is typing, for a person signed in
    /// without a voice to be known by. A voice, when there is one, is
    /// [`profile::SPEAKER`] and wins.
    pub static CHAT_PERSON: String;
}

/// Most workouts one answer lists: enough for a month of training, few
/// enough to read.
const MOST: usize = 40;

struct Shared {
    store: Arc<dyn TrainingStore>,
    tz: chrono_tz::Tz,
    imperial: bool,
}

fn invalid(tool: &str, reason: impl Into<String>) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason: reason.into(),
    }
}

/// Whose training this turn may read.
fn asker(tool: &str) -> Result<String> {
    if let Some(slug) = profile::SPEAKER.try_with(Clone::clone).ok().flatten() {
        return Ok(person_key(Some(&slug), ""));
    }
    if let Ok(person) = CHAT_PERSON.try_with(Clone::clone) {
        return Ok(person);
    }
    match approvals::asker() {
        Some(a) if !a.person.starts_with("satellite:") => Ok(a.person),
        _ => Err(invalid(
            tool,
            "I don't know who is asking, so I can't tell whose training this is. \
             Training data is each person's own, read only when Niles knows their voice \
             or they are signed in to the app",
        )),
    }
}

impl Shared {
    fn today(&self) -> NaiveDate {
        Utc::now().with_timezone(&self.tz).date_naive()
    }

    fn date_arg(&self, tool: &str, args: &Value, key: &str) -> Result<Option<NaiveDate>> {
        match args.get(key).and_then(Value::as_str).map(str::trim) {
            None | Some("") => Ok(None),
            Some(text) => {
                let date = text.get(..10).unwrap_or(text);
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .map(Some)
                    .map_err(|_| invalid(tool, format!("{key} must be YYYY-MM-DD, not {text:?}")))
            }
        }
    }

    /// `from..=to` in house days, defaulting to the `days` up to today.
    fn range(&self, tool: &str, args: &Value, days: i64) -> Result<(NaiveDate, NaiveDate)> {
        let to = self
            .date_arg(tool, args, "to")?
            .unwrap_or_else(|| self.today());
        let from = self
            .date_arg(tool, args, "from")?
            .unwrap_or(to - Duration::days(days - 1));
        if from > to {
            return Err(invalid(tool, "from is after to"));
        }
        Ok((from, to))
    }

    fn start_of(&self, day: NaiveDate) -> DateTime<Utc> {
        self.tz
            .from_local_datetime(&day.and_time(NaiveTime::MIN))
            .earliest()
            .map_or_else(|| day.and_time(NaiveTime::MIN).and_utc(), |t| t.to_utc())
    }

    fn distance(&self, metres: f64) -> Value {
        if self.imperial {
            json!({ "mi": round2(metres / 1609.344) })
        } else {
            json!({ "km": round2(metres / 1000.0) })
        }
    }

    /// Seconds per kilometre as "m:ss /km", or per mile in an imperial
    /// house.
    fn pace(&self, seconds_per_km: f64) -> String {
        let (per, unit) = if self.imperial {
            (seconds_per_km * 1.609_344, "mi")
        } else {
            (seconds_per_km, "km")
        };
        let s = per.round() as i64;
        format!("{}:{:02} /{unit}", s / 60, s % 60)
    }

    fn activity(&self, a: &Activity) -> Value {
        let local = a.started_at.with_timezone(&self.tz);
        let mut out = json!({
            "id": a.external_id,
            "date": local.format("%A %Y-%m-%d %H:%M").to_string(),
            "type": a.kind.as_str(),
            "name": a.name,
        });
        if let Some(m) = a.distance_m {
            out["distance"] = self.distance(m);
        }
        if let Some(s) = a.duration_s {
            out["duration"] = json!(clock(s));
        }
        if let (Some(m), Some(s)) = (a.distance_m, a.pace_seconds())
            && m > 0.0
            && matches!(a.kind, Kind::Run | Kind::Walk)
        {
            out["pace"] = json!(self.pace(s / (m / 1000.0)));
        }
        if let Some(hr) = a.avg_hr {
            out["avg_hr"] = json!(hr.round());
        }
        if let Some(hr) = a.max_hr {
            out["max_hr"] = json!(hr.round());
        }
        if let Some(m) = a.elevation_gain_m {
            out["elevation_gain"] = if self.imperial {
                json!({ "ft": (m / 0.3048).round() })
            } else {
                json!({ "m": m.round() })
            };
        }
        if let Some(kcal) = a.energy_kcal {
            out["kcal"] = json!(kcal.round());
        }
        out
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// "1:02:15", or "32:05" under an hour.
fn clock(seconds: f64) -> String {
    let s = seconds.round() as i64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn storage(e: niles_training::Error) -> Error {
    Error::Memory(e.to_string())
}

/// What to say when nothing has come from Strava yet.
const NOTHING_YET: &str = "No workouts have arrived for this person. They come from their own \
    Strava: on their profile page in the app, under Strava, they can connect it.";

struct TrainingActivities(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for TrainingActivities {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_activities".into(),
            description: "The workouts of the person asking — runs, walks, rides, strength — \
                from their Strava, oldest first: their Strava id, date, distance, time, pace, \
                heart rate, climb. from/to are YYYY-MM-DD in the house's time zone, to \
                inclusive; without them, the last 30 days."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "YYYY-MM-DD" },
                    "to": { "type": "string", "description": "YYYY-MM-DD, inclusive" },
                    "type": { "type": "string", "enum": ["run", "walk", "ride", "strength", "other"] }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "training_activities";
        let s = &self.0;
        let person = asker(TOOL)?;
        let (from, to) = s.range(TOOL, &args, 30)?;
        let kind = match args.get("type").and_then(Value::as_str) {
            None | Some("") => None,
            Some(t) => {
                Some(Kind::parse(t).ok_or_else(|| invalid(TOOL, format!("no such type {t:?}")))?)
            }
        };
        let found = s
            .store
            .activities(
                &person,
                Some(s.start_of(from)),
                Some(s.start_of(to + Duration::days(1))),
                kind,
            )
            .await
            .map_err(storage)?;
        if found.is_empty() && s.store.overview(&person).await.map_err(storage)?.activities == 0 {
            return Ok(json!({ "activities": [], "note": NOTHING_YET }));
        }
        let skipped = found.len().saturating_sub(MOST);
        let mut out = json!({
            "from": from.to_string(),
            "to": to.to_string(),
            "activities": found[skipped..].iter().map(|a| s.activity(a)).collect::<Vec<_>>(),
        });
        if skipped > 0 {
            out["earlier_not_shown"] = json!(skipped);
        }
        Ok(out)
    }
}

struct TrainingSummary(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for TrainingSummary {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_summary".into(),
            description: "How the person asking has been training, added up: runs, total \
                and longest distance, average pace, and all workouts. period is the \
                days up to today (week = 7, month = 30, quarter = 90, year = 365); from/to \
                (YYYY-MM-DD) instead for given dates. Use this for \"how was my week\"."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "period": { "type": "string", "enum": ["week", "month", "quarter", "year"] },
                    "from": { "type": "string", "description": "YYYY-MM-DD" },
                    "to": { "type": "string", "description": "YYYY-MM-DD, inclusive" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "training_summary";
        let s = &self.0;
        let person = asker(TOOL)?;
        let days = match args.get("period").and_then(Value::as_str).unwrap_or("week") {
            "week" => 7,
            "month" => 30,
            "quarter" => 90,
            "year" => 365,
            other => {
                return Err(invalid(
                    TOOL,
                    format!("period is week, month, quarter or year, not {other:?}"),
                ));
            }
        };
        let (from, to) = s.range(TOOL, &args, days)?;
        let activities = s
            .store
            .activities(
                &person,
                Some(s.start_of(from)),
                Some(s.start_of(to + Duration::days(1))),
                None,
            )
            .await
            .map_err(storage)?;
        if activities.is_empty()
            && s.store.overview(&person).await.map_err(storage)?.activities == 0
        {
            return Ok(json!({ "note": NOTHING_YET }));
        }
        let sum = niles_training::summarise(&activities, from, to, &s.tz);
        Ok(summary_json(s, &sum))
    }
}

fn summary_json(s: &Shared, sum: &niles_training::Summary) -> Value {
    let mut out = json!({
        "from": sum.from.to_string(),
        "to": sum.to.to_string(),
        "runs": sum.runs,
        "run_distance": s.distance(sum.run_distance_m),
        "run_time": clock(sum.run_seconds),
        "all_workouts": sum.activities,
    });
    if let Some((day, metres)) = sum.longest_run {
        out["longest_run"] = json!({
            "date": day.format("%A %Y-%m-%d").to_string(),
            "distance": s.distance(metres),
        });
    }
    if let Some(pace) = sum.pace_s_per_km {
        out["average_pace"] = json!(s.pace(pace));
    }
    out
}

/// What to say when there is no plan to read.
const NO_PLAN: &str = "This person has no training plan with a race still to come. Claude or \
    ChatGPT makes one when asked (\"make me a plan for the Copenhagen Half\"); it is kept under \
    Saved in the app, and follows their Strava runs.";

fn day_of(v: &Value) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(v.as_str()?, "%Y-%m-%d").ok()
}

/// "9 weeks to go", "10 days to go", "tomorrow", "race day".
fn countdown(days: i64) -> String {
    match days {
        ..0 => "the race has been run".into(),
        0 => "race day".into(),
        1 => "tomorrow".into(),
        2..14 => format!("{days} days to go"),
        _ => format!("{} weeks to go", days / 7),
    }
}

impl Shared {
    /// A plan's pace per km, "5:30-5:45", in the house's units.
    fn plan_pace(&self, raw: &str) -> Option<String> {
        let unit = if self.imperial { " /mi" } else { " /km" };
        let parts: Vec<String> = raw
            .split('-')
            .map(|p| {
                let (m, s) = p.trim().split_once(':')?;
                let seconds = m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?;
                Some(self.pace(seconds).trim_end_matches(unit).to_string())
            })
            .collect::<Option<_>>()?;
        Some(format!("{}{unit}", parts.join("-")))
    }

    /// A session as it is said: its day, what it is, how far and how
    /// fast, and what was run of it.
    fn session(&self, s: &Value) -> Value {
        let mut out = json!({
            "date": day_of(&s["date"]).map(|d| d.format("%A %Y-%m-%d").to_string()),
            "title": s["title"],
            "type": s["type"],
            "status": s["status"],
        });
        if let Some(km) = s["distance_km"].as_f64() {
            out["distance"] = self.distance(km * 1000.0);
        }
        if let Some(min) = s["duration_min"].as_f64() {
            out["duration"] = json!(clock(min * 60.0));
        }
        if let Some(pace) = s["pace"].as_str().and_then(|p| self.plan_pace(p)) {
            out["pace"] = json!(pace);
        }
        if let Some(details) = s["details"].as_str() {
            out["details"] = json!(details);
        }
        let actual = &s["actual"];
        if actual.is_object() {
            let mut ran = json!({});
            if let Some(km) = actual["distance_km"].as_f64() {
                ran["distance"] = self.distance(km * 1000.0);
            }
            if let Some(pace) = actual["pace"].as_str().and_then(|p| self.plan_pace(p)) {
                ran["pace"] = json!(pace);
            }
            out["ran"] = ran;
        }
        out
    }
}

/// Today's run, or another day's, from the asker's own plan.
struct TrainingToday {
    shared: Arc<Shared>,
    plans: Arc<dyn ArtifactStore>,
}

#[async_trait::async_trait]
impl Tool for TrainingToday {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_today".into(),
            description: "The training plan of the person asking, for one day: that day's \
                sessions, the next one after it, the week's distance planned and run so far, \
                and how long to the race. day is YYYY-MM-DD, today or tomorrow; today when \
                left out. Use it for \"what's my run today\", \"what's on my training plan\", \
                \"what run do I have tomorrow\"."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "day": { "type": "string", "description": "YYYY-MM-DD, today or tomorrow" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let person = asker(TrainingToday::TOOL)?;
        self.read(&person, &args, self.shared.today()).await
    }
}

impl TrainingToday {
    const TOOL: &str = "training_today";

    async fn read(&self, person: &str, args: &Value, today: NaiveDate) -> Result<Value> {
        const TOOL: &str = TrainingToday::TOOL;
        let s = &self.shared;
        let day = match args.get("day").and_then(Value::as_str).map(str::trim) {
            None | Some("" | "today") => today,
            Some("tomorrow") => today + Duration::days(1),
            Some(_) => s.date_arg(TOOL, args, "day")?.unwrap_or(today),
        };
        let found = plan_day::current_plan(self.plans.as_ref(), person, today)
            .await
            .map_err(|e| Error::Memory(e.to_string()))?;
        let Some((plan, content)) = found else {
            return Ok(json!({ "plan": null, "note": NO_PLAN }));
        };

        let d = PlanDay::of(&content, day, today);
        let mut out = json!({
            "plan": plan.title,
            "race": content["goal"]["race"],
            "race_day": d.race_day.format("%A %Y-%m-%d").to_string(),
            "days_to_go": d.days_to_go,
            "countdown": countdown(d.days_to_go),
            "day": day.format("%A %Y-%m-%d").to_string(),
            "sessions": d.sessions.iter().map(|x| s.session(x)).collect::<Vec<_>>(),
        });
        if d.rest_day() {
            out["note"] = json!("Nothing to run that day: a rest day.");
        }
        if let Some(next) = d.next {
            out["next"] = s.session(next);
        }
        if let Some(week) = &d.week {
            out["week"] = json!({
                "focus": week.focus,
                "planned": s.distance(week.planned_km * 1000.0),
                "run": s.distance(week.run_km * 1000.0),
                "sessions_left": week.sessions_left,
            });
        }
        Ok(out)
    }
}

/// `training_today`, beside the other training tools, reading the plans
/// in `plans`.
pub fn register_plan(
    reg: &mut ToolRegistry,
    store: Arc<dyn TrainingStore>,
    plans: Arc<dyn ArtifactStore>,
    tz: chrono_tz::Tz,
    imperial: bool,
) {
    let shared = Arc::new(Shared {
        store,
        tz,
        imperial,
    });
    reg.register(Box::new(TrainingToday { shared, plans }));
}

pub fn register(
    reg: &mut ToolRegistry,
    store: Arc<dyn TrainingStore>,
    tz: chrono_tz::Tz,
    imperial: bool,
) {
    let shared = Arc::new(Shared {
        store,
        tz,
        imperial,
    });
    reg.register(Box::new(TrainingActivities(shared.clone())));
    reg.register(Box::new(TrainingSummary(shared)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_training::MemoryTraining;

    async fn store_with_hers() -> Arc<MemoryTraining> {
        let store = Arc::new(MemoryTraining::new());
        let run = niles_training::strava::activity(&json!({
            "id": 1,
            "name": "Morning Run",
            "sport_type": "Run",
            "start_date": "2026-10-14T05:00:00Z",
            "distance": 5020.0,
            "moving_time": 1925,
            "elapsed_time": 1920,
            "average_heartrate": 150.0
        }))
        .unwrap();
        store.save_activities("person:majse", &[run]).await.unwrap();
        store
    }

    fn shared(store: Arc<MemoryTraining>) -> Arc<Shared> {
        Arc::new(Shared {
            store,
            tz: chrono_tz::Europe::Copenhagen,
            imperial: false,
        })
    }

    fn around_the_run() -> Value {
        json!({ "from": "2026-10-08", "to": "2026-10-14" })
    }

    #[tokio::test]
    async fn the_asker_sees_their_own_runs() {
        let tools = shared(store_with_hers().await);
        let out = profile::SPEAKER
            .scope(
                Some("majse".into()),
                TrainingActivities(tools).execute(around_the_run()),
            )
            .await
            .unwrap();
        let run = &out["activities"][0];
        assert_eq!(run["id"], "1");
        assert_eq!(run["type"], "run");
        assert_eq!(run["distance"]["km"], 5.02);
        assert_eq!(run["date"], "Wednesday 2026-10-14 07:00");
        assert_eq!(run["duration"], "32:00");
        assert_eq!(run["pace"], "6:23 /km");
    }

    #[tokio::test]
    async fn nobody_else_sees_them() {
        let tools = shared(store_with_hers().await);
        let out = profile::SPEAKER
            .scope(
                Some("mark".into()),
                TrainingActivities(tools.clone()).execute(around_the_run()),
            )
            .await
            .unwrap();
        assert_eq!(out["activities"], json!([]));
        assert!(out["note"].as_str().unwrap().contains("profile page"));

        let summary = CHAT_PERSON
            .scope(
                "email:guest@example.com".into(),
                TrainingSummary(tools).execute(around_the_run()),
            )
            .await
            .unwrap();
        assert!(summary.get("runs").is_none(), "{summary}");
    }

    #[tokio::test]
    async fn without_knowing_who_asks_there_is_a_polite_no() {
        let tools = shared(store_with_hers().await);
        let err = TrainingSummary(tools).execute(json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("don't know who is asking"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_job_for_somebody_signed_in_without_a_voice_reads_theirs() {
        let store = Arc::new(MemoryTraining::new());
        let run = niles_training::strava::activity(&json!({
            "id": 2, "name": "Lunch Run", "sport_type": "Run",
            "start_date": "2026-10-14T11:00:00Z", "distance": 8000.0, "moving_time": 2880
        }))
        .unwrap();
        store
            .save_activities("email:majse@example.com", &[run])
            .await
            .unwrap();
        let asker = |person: &str| approvals::Asker {
            person: person.into(),
            slug: None,
            satellite: None,
            conversation: None,
        };
        let tools = shared(store);
        let out = approvals::ASKER
            .scope(
                asker("email:majse@example.com"),
                TrainingSummary(tools.clone()).execute(around_the_run()),
            )
            .await
            .unwrap();
        assert_eq!(out["run_distance"]["km"], 8.0);

        let err = approvals::ASKER
            .scope(
                asker("satellite:10.0.0.9:5000"),
                TrainingSummary(tools).execute(around_the_run()),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("don't know who is asking"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_voice_wins_over_the_chat_s_address() {
        let tools = shared(store_with_hers().await);
        let out = CHAT_PERSON
            .scope(
                "email:majse@example.com".into(),
                profile::SPEAKER.scope(
                    Some("majse".into()),
                    TrainingActivities(tools).execute(around_the_run()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(out["activities"][0]["avg_hr"], 150.0);
    }

    #[tokio::test]
    async fn the_week_is_added_up_for_her() {
        let tools = shared(store_with_hers().await);
        let out = profile::SPEAKER
            .scope(
                Some("majse".into()),
                TrainingSummary(tools).execute(around_the_run()),
            )
            .await
            .unwrap();
        assert_eq!(out["runs"], 1);
        assert_eq!(out["run_distance"]["km"], 5.02);
        assert_eq!(out["longest_run"]["date"], "Wednesday 2026-10-14");
        assert_eq!(out["average_pace"], "6:23 /km");
    }

    /// Her plan toward a 10K on Sunday 25 October, and somebody else's
    /// plan, which is never read for her.
    async fn plans_with_hers() -> TrainingToday {
        use niles_artifacts::{Author, MemoryArtifacts, NewArtifact, NewRevision, Status};
        let plans = Arc::new(MemoryArtifacts::new());
        let content = json!({
            "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": "2026-10-25" },
            "weeks": [
                { "starts": "2026-10-12", "focus": "Build", "sessions": [
                    { "date": "2026-10-13", "type": "easy", "title": "Easy run", "distance_km": 6,
                      "status": "done", "activity": "1", "actual": { "distance_km": 6.4, "pace": "6:20" } },
                    { "date": "2026-10-16", "type": "intervals", "title": "6 × 400 m",
                      "distance_km": 7, "pace": "4:45-5:00", "details": "Warm up 10 min" },
                    { "date": "2026-10-17", "type": "rest", "title": "Rest" },
                    { "date": "2026-10-18", "type": "long", "title": "Long run", "distance_km": 12 }
                ]},
                { "starts": "2026-10-19", "sessions": [
                    { "date": "2026-10-25", "type": "race", "title": "Race day", "distance_km": 10 }
                ]}
            ]
        });
        for (person, title) in [
            ("person:mark", "His plan"),
            ("person:majse", "Hellerup 10K"),
        ] {
            plans
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
                            content: niles_artifacts::kinds::validate("training_plan", &content)
                                .unwrap(),
                            reason: "First draft".into(),
                            author: Author::Claude,
                            summary: None,
                            page: niles_artifacts::PageChange::Keep,
                        }),
                    },
                )
                .await
                .unwrap();
        }
        TrainingToday {
            shared: shared(Arc::new(MemoryTraining::new())),
            plans,
        }
    }

    fn on(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    async fn asked(tool: &TrainingToday, day: &str, today: NaiveDate) -> Value {
        tool.read("person:majse", &json!({ "day": day }), today)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn today_s_run_is_read_from_her_plan() {
        let tool = plans_with_hers().await;
        let out = asked(&tool, "today", on(16)).await;
        assert_eq!(out["plan"], "Hellerup 10K");
        assert_eq!(out["day"], "Friday 2026-10-16");
        let s = &out["sessions"][0];
        assert_eq!(s["title"], "6 × 400 m");
        assert_eq!(s["distance"]["km"], 7.0);
        assert_eq!(s["pace"], "4:45-5:00 /km");
        assert_eq!(s["details"], "Warm up 10 min");
        assert_eq!(out["next"]["title"], "Long run");
        assert_eq!(out["next"]["date"], "Sunday 2026-10-18");
        assert_eq!(out["week"]["planned"]["km"], 25.0);
        assert_eq!(out["week"]["run"]["km"], 6.4);
        assert_eq!(out["week"]["sessions_left"], 2);
        assert_eq!(out["race_day"], "Sunday 2026-10-25");
        assert!(out.get("note").is_none());
    }

    #[tokio::test]
    async fn a_rest_day_says_so_and_a_done_one_says_what_was_run() {
        let tool = plans_with_hers().await;
        let rest = asked(&tool, "tomorrow", on(16)).await;
        assert_eq!(rest["day"], "Saturday 2026-10-17");
        assert!(rest["note"].as_str().unwrap().contains("rest day"));
        let done = asked(&tool, "2026-10-13", on(16)).await;
        assert_eq!(done["sessions"][0]["status"], "done");
        assert_eq!(done["sessions"][0]["ran"]["distance"]["km"], 6.4);
        assert_eq!(done["sessions"][0]["ran"]["pace"], "6:20 /km");
    }

    #[test]
    fn the_countdown_is_said_as_people_say_it() {
        assert_eq!(countdown(63), "9 weeks to go");
        assert_eq!(countdown(10), "10 days to go");
        assert_eq!(countdown(1), "tomorrow");
        assert_eq!(countdown(0), "race day");
    }

    #[tokio::test]
    async fn without_a_plan_still_to_run_there_is_none() {
        let tool = plans_with_hers().await;
        let guest = tool.read("person:guest", &json!({}), on(16)).await.unwrap();
        assert_eq!(guest["plan"], Value::Null);
        assert!(guest["note"].as_str().unwrap().contains("no training plan"));
        let after = asked(&tool, "today", on(26)).await;
        assert_eq!(after["plan"], Value::Null, "the race is run");
        let his = profile::SPEAKER
            .scope(Some("mark".into()), tool.execute(json!({})))
            .await
            .unwrap();
        assert_ne!(his["plan"], "Hellerup 10K");
    }

    #[test]
    fn an_imperial_house_hears_miles() {
        let s = Shared {
            store: Arc::new(MemoryTraining::new()),
            tz: chrono_tz::UTC,
            imperial: true,
        };
        assert_eq!(s.distance(21_097.5), json!({ "mi": 13.11 }));
        assert_eq!(s.pace(300.0), "8:03 /mi");
        assert_eq!(s.plan_pace("5:00-5:30").as_deref(), Some("8:03-8:51 /mi"));
        assert_eq!(clock(5025.0), "1:23:45");
    }
}

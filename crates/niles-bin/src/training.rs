//! The training tools: the asker's own runs, sleep, heart rate and
//! weight, as their phone sent them.
//!
//! Whose comes from who is asking, never from an argument: the voice
//! [`profile::SPEAKER`] names, or in the app's chat the person signed
//! in ([`CHAT_PERSON`]). Somebody else's data cannot be asked for by
//! name, and an unrecognised voice has none.

use crate::profile;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use niles_training::{Activity, Kind, TrainingStore, Wellness, person_key};
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
    CHAT_PERSON.try_with(Clone::clone).map_err(|_| {
        invalid(
            tool,
            "I don't know who is asking, so I can't tell whose training this is. \
             Training data is each person's own, read only when Niles knows their voice \
             or they are signed in to the app",
        )
    })
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

    fn day(&self, w: &Wellness) -> Value {
        let mut out = json!({ "date": w.date.format("%A %Y-%m-%d").to_string() });
        if let Some(hr) = w.resting_hr {
            out["resting_hr"] = json!(hr.round());
        }
        if let Some(ms) = w.hrv_ms {
            out["hrv_ms"] = json!(ms.round());
        }
        if let Some(s) = w.sleep_s {
            out["sleep"] = json!(hours(s));
        }
        if let Some(kg) = w.weight_kg {
            out["weight"] = self.weight(kg);
        }
        if let Some(pct) = w.body_fat_pct {
            out["body_fat_pct"] = json!(round1(pct));
        }
        out
    }

    fn weight(&self, kg: f64) -> Value {
        if self.imperial {
            json!({ "lb": round1(kg / 0.453_592_37) })
        } else {
            json!({ "kg": round1(kg) })
        }
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
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

/// "7h 06m".
fn hours(seconds: f64) -> String {
    let m = (seconds / 60.0).round() as i64;
    format!("{}h {:02}m", m / 60, m % 60)
}

fn storage(e: niles_training::Error) -> Error {
    Error::Memory(e.to_string())
}

/// What to say when the phone has sent nothing yet.
const NOTHING_YET: &str = "No training data has arrived for this person. It comes from their \
    iPhone: on their profile page in the app, under Training, they can connect Health Auto Export.";

struct TrainingActivities(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for TrainingActivities {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_activities".into(),
            description: "The workouts of the person asking — runs, walks, rides, strength — \
                from their watch, oldest first: date, distance, time, pace, heart rate, \
                climb. from/to are YYYY-MM-DD in the house's time zone, to inclusive; \
                without them, the last 30 days."
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

struct TrainingWellness(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for TrainingWellness {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_wellness".into(),
            description: "The days of the person asking, from their watch and scale: resting \
                heart rate, HRV (ms), sleep, weight and body fat. A night's sleep is on the \
                day they woke up. from/to are YYYY-MM-DD, to inclusive; without them, the \
                last 14 days."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "YYYY-MM-DD" },
                    "to": { "type": "string", "description": "YYYY-MM-DD, inclusive" }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "training_wellness";
        let s = &self.0;
        let person = asker(TOOL)?;
        let (from, to) = s.range(TOOL, &args, 14)?;
        let days = s.store.wellness(&person, from, to).await.map_err(storage)?;
        if days.is_empty()
            && s.store
                .overview(&person)
                .await
                .map_err(storage)?
                .wellness_days
                == 0
        {
            return Ok(json!({ "days": [], "note": NOTHING_YET }));
        }
        Ok(json!({
            "from": from.to_string(),
            "to": to.to_string(),
            "days": days.iter().map(|w| s.day(w)).collect::<Vec<_>>(),
        }))
    }
}

struct TrainingSummary(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for TrainingSummary {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "training_summary".into(),
            description: "How the person asking has been training, added up: runs, total \
                and longest distance, average pace, all workouts, and resting heart rate, \
                HRV and sleep against the period before, and weight change. period is the \
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
        let length = (to - from).num_days() + 1;
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
        let wellness = s
            .store
            .wellness(&person, from - Duration::days(length), to)
            .await
            .map_err(storage)?;
        if activities.is_empty() && wellness.is_empty() {
            let overview = s.store.overview(&person).await.map_err(storage)?;
            if overview.activities == 0 && overview.wellness_days == 0 {
                return Ok(json!({ "note": NOTHING_YET }));
            }
        }
        let sum = niles_training::summarise(&activities, &wellness, from, to, &s.tz);
        Ok(summary_json(s, &sum))
    }
}

fn summary_json(s: &Shared, sum: &niles_training::Summary) -> Value {
    let trend = |t: &niles_training::Trend, show: &dyn Fn(f64) -> Value| {
        let mut out = json!({ "days_measured": t.days });
        if let Some(a) = t.average {
            out["average"] = show(a);
        }
        if let Some(p) = t.previous_average {
            out["period_before"] = show(p);
        }
        out
    };
    let mut out = json!({
        "from": sum.from.to_string(),
        "to": sum.to.to_string(),
        "runs": sum.runs,
        "run_distance": s.distance(sum.run_distance_m),
        "run_time": clock(sum.run_seconds),
        "all_workouts": sum.activities,
        "resting_hr": trend(&sum.resting_hr, &|v| json!(v.round())),
        "hrv_ms": trend(&sum.hrv_ms, &|v| json!(v.round())),
        "sleep_per_night": trend(&sum.sleep_s, &|v| json!(hours(v))),
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
    if let Some((day, kg)) = sum.weight.last {
        let mut weight = json!({
            "latest": s.weight(kg),
            "on": day.to_string(),
        });
        if let Some(change) = sum.weight.change_kg() {
            weight["change"] = s.weight(change);
        }
        out["weight"] = weight;
    }
    out
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
    reg.register(Box::new(TrainingWellness(shared.clone())));
    reg.register(Box::new(TrainingSummary(shared)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_training::MemoryTraining;

    async fn store_with_hers() -> Arc<MemoryTraining> {
        let store = Arc::new(MemoryTraining::new());
        let body = serde_json::from_str(include_str!(
            "../../niles-training/fixtures/hae-v2-metric.json"
        ))
        .unwrap();
        let batch = niles_training::hae::parse(&body, &chrono_tz::Europe::Copenhagen).unwrap();
        store
            .save_activities("person:majse", &batch.activities)
            .await
            .unwrap();
        store
            .save_wellness("person:majse", &batch.wellness)
            .await
            .unwrap();
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
    async fn a_voice_wins_over_the_chat_s_address() {
        let tools = shared(store_with_hers().await);
        let out = CHAT_PERSON
            .scope(
                "email:majse@example.com".into(),
                profile::SPEAKER.scope(
                    Some("majse".into()),
                    TrainingWellness(tools).execute(around_the_run()),
                ),
            )
            .await
            .unwrap();
        let day = out["days"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(day["resting_hr"], 54.0);
        assert_eq!(day["hrv_ms"], 49.0);
        assert_eq!(day["sleep"], "7h 06m");
        assert_eq!(day["weight"]["kg"], 61.2);
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
        assert_eq!(out["resting_hr"]["average"], 55.0);
        assert_eq!(out["weight"]["latest"]["kg"], 61.2);
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
        assert_eq!(clock(5025.0), "1:23:45");
        assert_eq!(hours(25_560.0), "7h 06m");
    }
}

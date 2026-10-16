//! A training plan: weeks of sessions toward a race, as JSON.
//!
//! Structured rather than markdown because Niles reads it as well as
//! the person: the app counts the weeks down to the race and adds up
//! each week's distance, and after each run Niles is to mark the session
//! it was (done, or missed) and link the Strava activity, which needs a
//! session with a date to point at. So every session has its date, type
//! and status from the first version on, and `activity` and `actual`
//! have their places before anything fills them.
//!
//! Strict, because the model is the one writing it and reads what is
//! wrong: an unknown field is refused by name rather than dropped, so a
//! misspelt `distance` is fixed instead of a plan quietly losing its
//! kilometres.

use crate::error::{Error, Result};
use crate::kinds::{Kind, Preview, PreviewLine, Style};
use chrono::{Datelike, Duration, NaiveDate};
use serde_json::{Map, Value, json};

pub(crate) const KIND: Kind = Kind::new(
    "training_plan",
    "Training plan",
    "{\"goal\": {\"race\", \"distance_km\", \"date\", \"target_time\"?}, \"weeks\": \
     [{\"starts\", \"focus\"?, \"sessions\": [{\"date\", \"type\", \"title\", \
     \"distance_km\"?, \"duration_min\"?, \"pace\"?, \"details\"?, \"status\"?}]}], \
     \"notes\"?, \"updates\"?}: a running plan toward a race, week by week. Dates are YYYY-MM-DD. The \
     first week starts on the plan's first day and every later week on the Monday after; \
     each session is dated inside its week, in order, and none after the race. type is \
     easy, long, tempo, intervals, recovery, race, rest, strength or cross. pace is per km, \
     \"5:40\" or \"5:30-5:45\"; target_time is \"1:55:00\". details is a line or two of \
     plain text (the reps, the effort); notes is markdown about the whole plan. status is \
     planned (the default), done, missed, skipped or moved; a done session may have \
     activity (its Strava id) and actual {distance_km, duration_min, pace, date?}, date \
     being the day it was run when that was not the session's. Keep both as they are when \
     changing a plan. updates is the coach's log, newest first: [{\"date\", \"note\"}], \
     a note of a sentence or three each time the plan is looked at after a run.",
    "Planning",
    validate,
    preview,
);

pub const TYPES: &[&str] = &[
    "easy",
    "long",
    "tempo",
    "intervals",
    "recovery",
    "race",
    "rest",
    "strength",
    "cross",
];

pub const STATUSES: &[&str] = &["planned", "done", "missed", "skipped", "moved"];

/// The types that are a run: the ones a run from Strava can be.
pub(crate) const RUNNING: &[&str] = &["easy", "long", "tempo", "intervals", "recovery", "race"];

/// A year: longer than any plan toward one race.
const MAX_WEEKS: usize = 52;
/// Two a day, every day.
const MAX_SESSIONS: usize = 14;
const MAX_NAME: usize = 80;
const MAX_DETAILS: usize = 1_000;
const MAX_NOTES: usize = 10_000;
/// The coach's log keeps the newest, the oldest going as a new one is
/// added, the way versions go: a plan looked at after every run for
/// months would otherwise grow without end.
const MAX_UPDATES: usize = 20;
const MAX_NOTE: usize = 500;
/// The whole plan as JSON: a year of full weeks with long details is
/// well under it.
const MAX_CHARS: usize = 400_000;
/// A session; a race may be an ultra.
pub(crate) const MAX_SESSION_KM: f64 = 100.0;
const MAX_RACE_KM: f64 = 250.0;
pub(crate) const MAX_MINUTES: f64 = 600.0;

/// Sessions a card shows, under the line about the race.
const PREVIEW_SESSIONS: usize = 3;

const GOAL: &[&str] = &["race", "distance_km", "date", "target_time"];
const PLAN: &[&str] = &["goal", "weeks", "notes", "updates"];
const WEEK: &[&str] = &["starts", "focus", "sessions"];
const SESSION: &[&str] = &[
    "date",
    "type",
    "title",
    "distance_km",
    "duration_min",
    "pace",
    "details",
    "status",
    "activity",
    "actual",
];
const ACTUAL: &[&str] = &["distance_km", "duration_min", "pace", "date"];
const UPDATE: &[&str] = &["date", "note"];

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "training plan",
        reason: reason.into(),
    }
}

/// A run that answered with the plan instead of saving it may have put
/// it in a code fence, or a sentence before it.
fn validate(content: &Value) -> Result<Value> {
    match content {
        Value::String(text) => plan(&from_text(text)?),
        other => plan(other),
    }
}

fn from_text(text: &str) -> Result<Value> {
    let text = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(text) {
        return Ok(v);
    }
    match (text.find('{'), text.rfind('}')) {
        (Some(open), Some(close)) if open < close => serde_json::from_str(&text[open..=close])
            .map_err(|e| {
                invalid(format!(
                    "content is a JSON object, and this is not one: {e}"
                ))
            }),
        _ => Err(invalid(
            "content is a JSON object, {\"goal\": {…}, \"weeks\": […]}, not text",
        )),
    }
}

fn plan(content: &Value) -> Result<Value> {
    let o = fields(content, "the plan", PLAN)?;
    let goal_value = present(o, "goal")
        .ok_or_else(|| invalid("the plan needs \"goal\": {\"race\", \"distance_km\", \"date\"}"))?;
    let (goal, race_day) = goal(goal_value)?;

    let weeks = present(o, "weeks")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("the plan needs \"weeks\", a list of weeks"))?;
    if weeks.is_empty() {
        return Err(invalid("the plan has no weeks"));
    }
    if weeks.len() > MAX_WEEKS {
        return Err(invalid(format!(
            "the plan has {} weeks, and at most {MAX_WEEKS} are kept",
            weeks.len()
        )));
    }
    let mut kept = Vec::with_capacity(weeks.len());
    let mut next_monday: Option<NaiveDate> = None;
    for (i, w) in weeks.iter().enumerate() {
        let (week, ends) = week(w, i + 1, next_monday, race_day)?;
        next_monday = Some(ends + Duration::days(1));
        kept.push(week);
    }

    let mut out = json!({ "goal": goal, "weeks": kept });
    if let Some(notes) = text(o, "notes", "the plan", MAX_NOTES)? {
        out["notes"] = json!(notes);
    }
    if let Some(list) = present(o, "updates") {
        let kept = updates(list)?;
        if !kept.is_empty() {
            out["updates"] = json!(kept);
        }
    }
    let n = out.to_string().len();
    if n > MAX_CHARS {
        return Err(invalid(format!(
            "the plan is {n} characters as JSON, and at most {MAX_CHARS} are kept: shorten \
             the details"
        )));
    }
    Ok(out)
}

fn goal(value: &Value) -> Result<(Value, NaiveDate)> {
    const AT: &str = "goal";
    let o = fields(value, AT, GOAL)?;
    let race = name(o, "race", AT)?
        .ok_or_else(|| invalid("goal needs \"race\", its name: \"Copenhagen Half Marathon\""))?;
    let distance = number(o, "distance_km", AT, MAX_RACE_KM)?
        .ok_or_else(|| invalid("goal needs \"distance_km\": 21.1 for a half marathon"))?;
    let day = date(o, "date", AT)?.ok_or_else(|| invalid("goal needs \"date\", the race day"))?;
    let mut out = json!({ "race": race, "distance_km": distance, "date": day.to_string() });
    if let Some(raw) = string(o, "target_time", AT)? {
        out["target_time"] = json!(finish_time(&raw).ok_or_else(|| invalid(format!(
            "goal: target_time is h:mm:ss, such as \"1:55:00\", not {raw:?}"
        )))?);
    }
    Ok((out, day))
}

/// One week, and the day it ends: the Sunday it reaches.
fn week(
    value: &Value,
    n: usize,
    must_start: Option<NaiveDate>,
    race_day: NaiveDate,
) -> Result<(Value, NaiveDate)> {
    let at = format!("week {n}");
    let o = fields(value, &at, WEEK)?;
    let starts = date(o, "starts", &at)?
        .ok_or_else(|| invalid(format!("{at} needs \"starts\", the day it begins")))?;
    let at = format!("week {n} ({starts})");
    match must_start {
        Some(monday) if starts != monday => {
            return Err(invalid(format!(
                "{at} must start on {monday}, the Monday after week {} began: weeks follow one \
                 another with no gap and no overlap, and only the first may start on another \
                 day than a Monday",
                n - 1
            )));
        }
        None if starts > race_day => {
            return Err(invalid(format!(
                "the plan starts on {starts}, after the race on {race_day}"
            )));
        }
        _ if starts > race_day => {
            return Err(invalid(format!(
                "{at} starts after the race on {race_day}: the plan ends with the race week"
            )));
        }
        _ => {}
    }
    let ends = starts + Duration::days(i64::from(6 - starts.weekday().num_days_from_monday()));

    let sessions = present(o, "sessions")
        .map(|v| {
            v.as_array()
                .ok_or_else(|| invalid(format!("{at}: sessions is a list")))
        })
        .transpose()?
        .map_or(&[][..], Vec::as_slice);
    if sessions.len() > MAX_SESSIONS {
        return Err(invalid(format!(
            "{at} has {} sessions, and at most {MAX_SESSIONS} are kept",
            sessions.len()
        )));
    }
    let mut kept = Vec::with_capacity(sessions.len());
    let mut last: Option<NaiveDate> = None;
    for (i, s) in sessions.iter().enumerate() {
        let (session, day) = session(s, &format!("{at}, session {}", i + 1))?;
        let at = format!("{at}, session {} ({day})", i + 1);
        if day < starts || day > ends {
            return Err(invalid(format!(
                "{at} is not in its week, which runs {starts} to {ends}"
            )));
        }
        if day > race_day {
            return Err(invalid(format!("{at} is after the race on {race_day}")));
        }
        if last.is_some_and(|last| day < last) {
            return Err(invalid(format!(
                "{at} comes before the session listed above it: a week's sessions are in the \
                 order of their dates"
            )));
        }
        last = Some(day);
        kept.push(session);
    }

    let mut out = json!({ "starts": starts.to_string(), "sessions": kept });
    if let Some(focus) = name(o, "focus", &at)? {
        out["focus"] = json!(focus);
    }
    Ok((out, ends))
}

fn session(value: &Value, at: &str) -> Result<(Value, NaiveDate)> {
    let o = fields(value, at, SESSION)?;
    let day = date(o, "date", at)?.ok_or_else(|| invalid(format!("{at} needs \"date\"")))?;
    let at = format!("{at} ({day})");
    let kind = one_of(o, "type", &at, TYPES)?
        .ok_or_else(|| invalid(format!("{at} needs \"type\": {}", TYPES.join(", "))))?;
    let title = name(o, "title", &at)?
        .ok_or_else(|| invalid(format!("{at} needs \"title\": \"Easy run\"")))?;
    let status = one_of(o, "status", &at, STATUSES)?.unwrap_or("planned");

    let mut out = json!({
        "date": day.to_string(),
        "type": kind,
        "title": title,
        "status": status,
    });
    measures(o, &at, &mut out)?;
    if let Some(details) = text(o, "details", &at, MAX_DETAILS)? {
        out["details"] = json!(details);
    }

    let activity = activity(o, &at)?;
    let actual = match present(o, "actual") {
        None => None,
        Some(v) => {
            let at = format!("{at}: actual");
            let a = fields(v, &at, ACTUAL)?;
            let mut actual = json!({});
            measures(a, &at, &mut actual)?;
            if let Some(day) = date(a, "date", &at)? {
                actual["date"] = json!(day.to_string());
            }
            actual
                .as_object()
                .is_some_and(|m| !m.is_empty())
                .then_some(actual)
        }
    };
    if status != "done" && (activity.is_some() || actual.is_some()) {
        return Err(invalid(format!(
            "{at} is {status}, and only a session that is done has an activity or actual"
        )));
    }
    if let Some(activity) = activity {
        out["activity"] = json!(activity);
    }
    if let Some(actual) = actual {
        out["actual"] = actual;
    }
    Ok((out, day))
}

/// The coach's log, newest first whatever order it came in, without
/// the empty notes, and only the newest [`MAX_UPDATES`] of it.
fn updates(value: &Value) -> Result<Vec<Value>> {
    let list = value
        .as_array()
        .ok_or_else(|| invalid("updates is a list, [{\"date\", \"note\"}]"))?;
    let mut kept = Vec::with_capacity(list.len());
    for (i, u) in list.iter().enumerate() {
        let at = format!("updates, note {}", i + 1);
        let o = fields(u, &at, UPDATE)?;
        let day = date(o, "date", &at)?
            .ok_or_else(|| invalid(format!("{at} needs \"date\", the day it was written")))?;
        if let Some(note) = text(o, "note", &at, MAX_NOTE)? {
            kept.push((day, note));
        }
    }
    kept.sort_by_key(|u| std::cmp::Reverse(u.0));
    kept.truncate(MAX_UPDATES);
    Ok(kept
        .into_iter()
        .map(|(day, note)| json!({ "date": day.to_string(), "note": note }))
        .collect())
}

/// Distance, duration and pace, as a session plans them or a run did
/// them.
fn measures(o: &Map<String, Value>, at: &str, out: &mut Value) -> Result<()> {
    if let Some(km) = number(o, "distance_km", at, MAX_SESSION_KM)? {
        out["distance_km"] = json!(km);
    }
    if let Some(min) = number(o, "duration_min", at, MAX_MINUTES)? {
        out["duration_min"] = json!(min);
    }
    if let Some(raw) = string(o, "pace", at)? {
        out["pace"] = json!(pace(&raw).ok_or_else(|| invalid(format!(
            "{at}: pace is minutes per km, \"5:40\" or \"5:30-5:45\", not {raw:?}"
        )))?);
    }
    Ok(())
}

/// The source's own id for the workout, as `niles-training` keeps it: a
/// number from Strava, kept as text.
fn activity(o: &Map<String, Value>, at: &str) -> Result<Option<String>> {
    let id = match present(o, "activity") {
        None => return Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(|n| n.to_string()),
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => None,
    };
    match id {
        Some(id) if !id.is_empty() && id.len() <= 40 && !id.contains(char::is_whitespace) => {
            Ok(Some(id))
        }
        _ => Err(invalid(format!(
            "{at}: activity is the workout's Strava id, such as 12345678901"
        ))),
    }
}

/// An object's fields, refusing any it does not have.
fn fields<'a>(value: &'a Value, at: &str, known: &[&str]) -> Result<&'a Map<String, Value>> {
    let o = value
        .as_object()
        .ok_or_else(|| invalid(format!("{at} is an object, {{\"{}\": …}}", known[0])))?;
    if let Some(k) = o.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(invalid(format!(
            "{at} has {k:?}, which is not part of a training plan; its fields are {}",
            known.join(", ")
        )));
    }
    Ok(o)
}

/// A field that is there: `null` is as good as left out.
fn present<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

fn string(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    match present(o, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(format!("{at}: {key} is text"))),
    }
}

/// One line of text, or nothing when it is empty.
fn name(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > MAX_NAME {
        return Err(invalid(format!(
            "{at}: {key} is longer than {MAX_NAME} characters"
        )));
    }
    Ok((!line.is_empty()).then_some(line))
}

/// Text that may run to lines, tidied as a document's is.
fn text(o: &Map<String, Value>, key: &str, at: &str, max: usize) -> Result<Option<String>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let tidy: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let tidy = tidy.trim();
    let n = tidy.chars().count();
    if n > max {
        return Err(invalid(format!(
            "{at}: {key} is {n} characters, and at most {max} are kept"
        )));
    }
    Ok((!tidy.is_empty()).then(|| tidy.to_string()))
}

fn one_of(
    o: &Map<String, Value>,
    key: &str,
    at: &str,
    allowed: &[&'static str],
) -> Result<Option<&'static str>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let raw = raw.trim().to_lowercase();
    allowed
        .iter()
        .find(|a| **a == raw)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            invalid(format!(
                "{at}: {key} is one of {}, not {raw:?}",
                allowed.join(", ")
            ))
        })
}

/// A positive number up to `max`, to two decimals. Written as text it
/// is still the number.
fn number(o: &Map<String, Value>, key: &str, at: &str, max: f64) -> Result<Option<f64>> {
    let Some(v) = present(o, key) else {
        return Ok(None);
    };
    let x = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| invalid(format!("{at}: {key} is a number, such as 6.5")))?;
    if !(x > 0.0 && x <= max) {
        return Err(invalid(format!(
            "{at}: {key} is {x}, and it is more than 0 and at most {max}"
        )));
    }
    Ok(Some((x * 100.0).round() / 100.0))
}

fn date(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<NaiveDate>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let raw = raw.trim();
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .filter(|_| raw.len() == 10)
        .map(Some)
        .ok_or_else(|| invalid(format!("{at}: {key} is a date, YYYY-MM-DD, not {raw:?}")))
}

/// "5:40", or a range "5:30-5:45" with the faster first; "/km" after it
/// is allowed and dropped.
pub(crate) fn pace(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let raw = ["min/km", "/km", "per km"]
        .iter()
        .find_map(|unit| raw.strip_suffix(unit))
        .unwrap_or(raw)
        .trim();
    let mut parts: Vec<u32> = raw
        .split(['-', '–'])
        .map(|p| min_sec(p.trim()))
        .collect::<Option<_>>()?;
    if parts.len() > 2 {
        return None;
    }
    parts.sort_unstable();
    Some(
        parts
            .iter()
            .map(|s| format!("{}:{:02}", s / 60, s % 60))
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// "m:ss" per km, between two and twenty minutes, in seconds.
fn min_sec(raw: &str) -> Option<u32> {
    let (m, s) = raw.split_once(':')?;
    if s.len() != 2 {
        return None;
    }
    let (m, s): (u32, u32) = (m.parse().ok()?, s.parse().ok()?);
    ((2..=20).contains(&m) && s < 60).then_some(m * 60 + s)
}

/// "1:55:00", or "55:00" under an hour, kept as h:mm:ss.
fn finish_time(raw: &str) -> Option<String> {
    let parts: Vec<u32> = raw
        .trim()
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let (h, m, s) = match parts[..] {
        [h, m, s] => (h, m, s),
        [m, s] => (0, m, s),
        _ => return None,
    };
    (h < 24 && m < 60 && s < 60 && h + m + s > 0).then(|| format!("{h}:{m:02}:{s:02}"))
}

fn preview(content: &Value) -> Preview {
    let goal = &content["goal"];
    let weeks = content["weeks"].as_array().map_or(&[][..], Vec::as_slice);
    let mut head = Vec::new();
    if let Some(race) = goal["race"].as_str() {
        head.push(race.to_string());
    }
    if let Some(day) = goal["date"].as_str().and_then(parse_date) {
        head.push(day.format("%a %-d %b").to_string());
    }
    if !weeks.is_empty() {
        head.push(match weeks.len() {
            1 => "1 week".to_string(),
            n => format!("{n} weeks"),
        });
    }
    let mut lines = Vec::new();
    if !head.is_empty() {
        lines.push(PreviewLine {
            text: head.join(" · "),
            style: Style::Heading,
            checked: None,
        });
    }
    let next = weeks
        .iter()
        .filter_map(|w| w["sessions"].as_array())
        .flatten()
        .filter(|s| s["status"] == "planned" && s["type"] != "rest")
        .take(PREVIEW_SESSIONS);
    lines.extend(next.map(|s| PreviewLine {
        text: session_line(s),
        style: Style::Bullet,
        checked: None,
    }));
    Preview { lines }
}

/// "Tue 20 Oct · Easy run · 6 km".
pub(crate) fn session_line(s: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(day) = s["date"].as_str().and_then(parse_date) {
        parts.push(day.format("%a %-d %b").to_string());
    }
    if let Some(title) = s["title"].as_str() {
        parts.push(title.to_string());
    }
    if let Some(km) = s["distance_km"].as_f64() {
        parts.push(format!("{} km", trim_number(km)));
    } else if let Some(min) = s["duration_min"].as_f64() {
        parts.push(format!("{} min", trim_number(min)));
    }
    parts.join(" · ")
}

pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// 6 as "6", 21.1 as "21.1".
fn trim_number(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two weeks toward a 10 km race: the first starts on a Thursday.
    fn plan_json() -> Value {
        json!({
            "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": "2026-10-25", "target_time": "55:00" },
            "weeks": [
                { "starts": "2026-10-15", "focus": "  Getting going ", "sessions": [
                    { "date": "2026-10-15", "type": "easy", "title": "Easy run", "distance_km": 5,
                      "pace": "6:30 /km", "status": "done", "activity": 12345678901u64,
                      "actual": { "distance_km": 5.02, "duration_min": 32, "pace": "6:23" } },
                    { "date": "2026-10-17", "type": "rest", "title": "Rest" },
                    { "date": "2026-10-18", "type": "long", "title": "Long run", "distance_km": "8" }
                ]},
                { "starts": "2026-10-19", "sessions": [
                    { "date": "2026-10-21", "type": "Intervals", "title": "6 × 400 m",
                      "duration_min": 40, "pace": "5:00-4:45", "details": "Warm up\r\n10 min" },
                    { "date": "2026-10-25", "type": "race", "title": "Race day", "distance_km": 10 }
                ]}
            ],
            "notes": "Three runs a week, as you asked."
        })
    }

    fn refused(content: Value) -> String {
        validate(&content).unwrap_err().to_string()
    }

    #[test]
    fn a_plan_is_kept_with_its_values_tidied() {
        let v = validate(&plan_json()).unwrap();
        assert_eq!(v["goal"]["target_time"], "0:55:00");
        assert_eq!(v["goal"]["distance_km"], 10.0);
        let first = &v["weeks"][0];
        assert_eq!(first["focus"], "Getting going");
        assert_eq!(first["sessions"][0]["pace"], "6:30");
        assert_eq!(first["sessions"][0]["activity"], "12345678901");
        assert_eq!(first["sessions"][0]["actual"]["pace"], "6:23");
        assert_eq!(first["sessions"][1]["status"], "planned");
        assert_eq!(first["sessions"][2]["distance_km"], 8.0);
        let intervals = &v["weeks"][1]["sessions"][0];
        assert_eq!(intervals["type"], "intervals");
        assert_eq!(intervals["pace"], "4:45-5:00");
        assert_eq!(intervals["details"], "Warm up\n10 min");
        assert_eq!(v["notes"], "Three runs a week, as you asked.");
    }

    #[test]
    fn a_kept_plan_is_kept_again_unchanged() {
        let once = validate(&plan_json()).unwrap();
        assert_eq!(validate(&once).unwrap(), once);
    }

    #[test]
    fn a_plan_answered_as_text_is_read_out_of_it() {
        let text = format!("Here it is:\n```json\n{}\n```", plan_json());
        let v = validate(&Value::String(text)).unwrap();
        assert_eq!(v["goal"]["race"], "Hellerup 10K");
        assert!(refused(json!("A plan with easy runs.")).contains("JSON object"));
    }

    #[test]
    fn an_unknown_field_is_refused_by_name() {
        let mut p = plan_json();
        p["weeks"][1]["sessions"][1]["distance"] = json!(10);
        let e = refused(p);
        assert!(e.contains("week 2 (2026-10-19), session 2 has"), "{e}");
        assert!(e.contains("\"distance\""), "{e}");
        assert!(e.contains("distance_km"), "{e}");
    }

    #[test]
    fn null_is_the_same_as_left_out() {
        let mut p = plan_json();
        p["weeks"][1]["sessions"][1]["pace"] = Value::Null;
        p["notes"] = Value::Null;
        let v = validate(&p).unwrap();
        assert!(v.get("notes").is_none());
        assert!(v["weeks"][1]["sessions"][1].get("pace").is_none());
    }

    #[test]
    fn the_goal_needs_a_race_a_distance_and_a_day() {
        let mut p = plan_json();
        p["goal"].as_object_mut().unwrap().remove("date");
        assert!(refused(p).contains("race day"));
        let mut p = plan_json();
        p["goal"]["distance_km"] = json!(0);
        assert!(refused(p).contains("more than 0"));
        let mut p = plan_json();
        p["goal"]["target_time"] = json!("fast");
        assert!(refused(p).contains("h:mm:ss"));
    }

    #[test]
    fn dates_are_iso_days() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][0]["date"] = json!("15/10/2026");
        assert!(refused(p).contains("YYYY-MM-DD"));
        let mut p = plan_json();
        p["goal"]["date"] = json!("2026-02-30");
        assert!(refused(p).contains("YYYY-MM-DD"));
    }

    #[test]
    fn weeks_follow_one_another_from_monday_to_sunday() {
        let mut p = plan_json();
        p["weeks"][1]["starts"] = json!("2026-10-20");
        let e = refused(p);
        assert!(e.contains("must start on 2026-10-19"), "{e}");

        let mut p = plan_json();
        p["weeks"][1]["starts"] = json!("2026-10-26");
        assert!(refused(p).contains("must start on 2026-10-19"));
    }

    #[test]
    fn a_session_is_inside_its_week_and_in_order() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][2]["date"] = json!("2026-10-19");
        let e = refused(p);
        assert!(e.contains("runs 2026-10-15 to 2026-10-18"), "{e}");

        let mut p = plan_json();
        p["weeks"][0]["sessions"][1]["date"] = json!("2026-10-14");
        assert!(refused(p).contains("not in its week"));

        let mut p = plan_json();
        p["weeks"][0]["sessions"][2]["date"] = json!("2026-10-16");
        assert!(refused(p).contains("order of their dates"));
    }

    #[test]
    fn nothing_comes_after_the_race() {
        let mut p = plan_json();
        p["goal"]["date"] = json!("2026-10-24");
        assert!(refused(p).contains("after the race on 2026-10-24"));

        let mut p = plan_json();
        p["goal"]["date"] = json!("2026-10-18");
        let e = refused(p);
        assert!(
            e.contains("week 2") && e.contains("starts after the race"),
            "{e}"
        );

        let mut p = plan_json();
        p["goal"]["date"] = json!("2026-10-01");
        assert!(refused(p).contains("the plan starts on 2026-10-15, after the race"));
    }

    #[test]
    fn a_type_or_status_it_does_not_know_lists_the_ones_it_does() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][0]["type"] = json!("fartlek");
        let e = refused(p);
        assert!(e.contains("easy, long, tempo"), "{e}");

        let mut p = plan_json();
        p["weeks"][0]["sessions"][1]["status"] = json!("finished");
        assert!(refused(p).contains("planned, done, missed"));
    }

    #[test]
    fn only_a_done_session_has_what_was_run() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][0]["status"] = json!("missed");
        let e = refused(p);
        assert!(
            e.contains("is missed, and only a session that is done"),
            "{e}"
        );
    }

    #[test]
    fn numbers_and_paces_are_sane() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][2]["distance_km"] = json!(120);
        assert!(refused(p).contains("at most 100"));
        let mut p = plan_json();
        p["weeks"][0]["sessions"][2]["duration_min"] = json!(-5);
        assert!(refused(p).contains("more than 0"));
        for bad in ["fast", "5:7", "1:30", "5:60", "4:00-4:30-5:00"] {
            let mut p = plan_json();
            p["weeks"][0]["sessions"][2]["pace"] = json!(bad);
            assert!(refused(p).contains("minutes per km"), "{bad}");
        }
    }

    #[test]
    fn a_runaway_plan_is_refused() {
        let mut p = plan_json();
        let mut weeks = vec![p["weeks"][0].clone()];
        weeks.extend(std::iter::repeat_n(p["weeks"][1].clone(), MAX_WEEKS));
        p["weeks"] = json!(weeks);
        assert!(refused(p).contains("at most 52"));

        let mut p = plan_json();
        let rest = json!({ "date": "2026-10-25", "type": "rest", "title": "Rest" });
        p["weeks"][1]["sessions"] = json!(vec![rest; MAX_SESSIONS + 1]);
        assert!(refused(p).contains("at most 14"));

        let mut p = plan_json();
        p["notes"] = json!("a".repeat(MAX_NOTES + 1));
        assert!(refused(p).contains("at most 10000"));
    }

    #[test]
    fn a_plan_needs_its_goal_and_a_week() {
        assert!(refused(json!({ "weeks": [] })).contains("goal"));
        let mut p = plan_json();
        p["weeks"] = json!([]);
        assert!(refused(p).contains("no weeks"));
        assert!(refused(json!(42)).contains("object"));
    }

    #[test]
    fn the_coach_s_log_is_kept_newest_first_and_only_the_newest() {
        let mut p = plan_json();
        let mut notes: Vec<Value> = (1..=25)
            .map(|d| json!({ "date": format!("2026-09-{d:02}"), "note": format!("Note {d}") }))
            .collect();
        notes.push(json!({ "date": "2026-10-01", "note": "  " }));
        p["updates"] = json!(notes);
        let v = validate(&p).unwrap();
        let kept = v["updates"].as_array().unwrap();
        assert_eq!(kept.len(), MAX_UPDATES);
        assert_eq!(kept[0], json!({ "date": "2026-09-25", "note": "Note 25" }));
        assert_eq!(kept[MAX_UPDATES - 1]["note"], "Note 6");

        let mut p = plan_json();
        p["updates"] = json!([{ "date": "2026-10-15", "text": "Good run" }]);
        assert!(refused(p).contains("\"text\""));
        let mut p = plan_json();
        p["updates"] = json!([{ "date": "2026-10-15", "note": "x".repeat(MAX_NOTE + 1) }]);
        assert!(refused(p).contains("at most 500"));
        let mut p = plan_json();
        p["updates"] = json!([]);
        assert!(validate(&p).unwrap().get("updates").is_none());
    }

    #[test]
    fn a_run_done_on_another_day_keeps_the_day_it_was_run() {
        let mut p = plan_json();
        p["weeks"][0]["sessions"][0]["actual"]["date"] = json!("2026-10-16");
        let v = validate(&p).unwrap();
        assert_eq!(v["weeks"][0]["sessions"][0]["actual"]["date"], "2026-10-16");
        p["weeks"][0]["sessions"][0]["actual"]["date"] = json!("Friday");
        assert!(refused(p).contains("YYYY-MM-DD"));
    }

    #[test]
    fn the_preview_is_the_race_and_the_next_sessions_not_yet_run() {
        let v = validate(&plan_json()).unwrap();
        let lines: Vec<(Style, String)> = preview(&v)
            .lines
            .into_iter()
            .map(|l| (l.style, l.text))
            .collect();
        assert_eq!(
            lines,
            vec![
                (Style::Heading, "Hellerup 10K · Sun 25 Oct · 2 weeks".into()),
                (Style::Bullet, "Sun 18 Oct · Long run · 8 km".into()),
                (Style::Bullet, "Wed 21 Oct · 6 × 400 m · 40 min".into()),
                (Style::Bullet, "Sun 25 Oct · Race day · 10 km".into()),
            ]
        );
    }
}

//! A training plan kept up with the runs: after each sync, the workouts
//! that came in are matched to the sessions they were, and the sessions
//! whose day went by without one are marked missed. In code, without a
//! model: what was run is a fact, and saying it needs no judgement.
//!
//! Then the one rule a model's review of the plan is held to, whatever
//! it was told: what has happened stays as it happened
//! ([`keeps_the_past`]).
//!
//! Days are the house's days, worked out by the caller, who also says
//! which day today is: nothing here reads a clock.

use crate::error::{Error, Result};
use crate::training_plan::{MAX_MINUTES, MAX_SESSION_KM, RUNNING, pace, parse_date, session_line};
use chrono::{Duration, NaiveDate};
use serde_json::{Value, json};
use std::collections::HashSet;

/// How far from its session's day a run may be and still be that
/// session: a day or two late, or early, as people move a run around
/// the weather.
pub const NEAR_DAYS: i64 = 2;

/// A session is missed only once the day after it has ended too, so a
/// watch that uploads the next morning still finds it planned.
pub const GRACE_DAYS: i64 = 1;

/// What a workout can stand in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sport {
    /// Any running session: easy, long, tempo, intervals, recovery, the
    /// race.
    Run,
    Strength,
    /// A ride, a swim, yoga: cross-training.
    Cross,
}

impl Sport {
    fn fits(self, session_type: &str) -> bool {
        match self {
            Sport::Run => RUNNING.contains(&session_type),
            Sport::Strength => session_type == "strength",
            Sport::Cross => session_type == "cross",
        }
    }
}

/// One workout, as a session's `actual` will keep it.
#[derive(Debug, Clone, PartialEq)]
pub struct Workout {
    /// The source's id, as a session's `activity` keeps it.
    pub id: String,
    /// The day it was done, in the house's time zone.
    pub day: NaiveDate,
    pub sport: Sport,
    pub distance_km: Option<f64>,
    pub duration_min: Option<f64>,
    /// Per km, "m:ss".
    pub pace: Option<String>,
}

/// A session a workout was matched to.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// The session as it was planned, before it was marked.
    pub session: Value,
    pub workout: Workout,
}

/// What the runs did to a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    /// The plan with the sessions marked, to be checked by its kind
    /// before it is kept, as any version is.
    pub content: Value,
    pub done: Vec<Done>,
    /// The sessions marked missed, as they were planned.
    pub missed: Vec<Value>,
}

impl Tracked {
    /// For the list of versions: "Done: Easy run on Tue 20 Oct. Missed:
    /// Long run on Sun 18 Oct".
    pub fn reason(&self) -> String {
        let on = |s: &Value| {
            let title = s["title"].as_str().unwrap_or("session");
            match s["date"].as_str().and_then(parse_date) {
                Some(day) => format!("{title} on {}", day.format("%a %-d %b")),
                None => title.to_string(),
            }
        };
        let mut parts = Vec::new();
        if !self.done.is_empty() {
            let list: Vec<String> = self.done.iter().map(|d| on(&d.session)).collect();
            parts.push(format!("Done: {}", list.join(", ")));
        }
        if !self.missed.is_empty() {
            let list: Vec<String> = self.missed.iter().map(on).collect();
            parts.push(format!("Missed: {}", list.join(", ")));
        }
        parts.join(". ")
    }
}

/// Where a session is in the plan, and what matching needs of it.
struct Slot {
    week: usize,
    n: usize,
    day: NaiveDate,
    kind: String,
    status: String,
    linked: bool,
}

fn slots(content: &Value) -> Vec<Slot> {
    let mut out = Vec::new();
    let weeks = content["weeks"].as_array().map_or(&[][..], Vec::as_slice);
    for (week, w) in weeks.iter().enumerate() {
        let sessions = w["sessions"].as_array().map_or(&[][..], Vec::as_slice);
        for (n, s) in sessions.iter().enumerate() {
            let Some(day) = s["date"].as_str().and_then(parse_date) else {
                continue;
            };
            out.push(Slot {
                week,
                n,
                day,
                kind: s["type"].as_str().unwrap_or_default().to_string(),
                status: s["status"].as_str().unwrap_or("planned").to_string(),
                linked: s.get("activity").is_some_and(|a| !a.is_null()),
            });
        }
    }
    out
}

/// The plan with `workouts` matched to its sessions and the sessions
/// gone by marked missed; `None` when nothing changes.
///
/// - A workout already linked to a session is never matched again, so
///   a re-sync or a backfill changes nothing it changed before.
/// - On its own day first: the first session there of a type it can
///   stand in for that is planned, missed (a late upload after the day
///   was given up on) or done by hand without a workout yet. Never one
///   the person skipped or that was moved.
/// - Only then a planned or missed session up to [`NEAR_DAYS`] away,
///   the nearest, the earlier on a tie: a run moved a day. Never the
///   race, which is on its day or not at all. Only for a
///   workout on a day that had nothing of its kind planned; on a day
///   that did, a second run is an extra one, and does not take the next
///   day's session from the run still to come.
/// - Then every planned session other than a rest day whose day and
///   the [`GRACE_DAYS`] after it are over is missed.
pub fn track(content: &Value, workouts: &[Workout], today: NaiveDate) -> Option<Tracked> {
    let slots = slots(content);
    let linked: HashSet<&str> = content["weeks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["sessions"].as_array())
        .flatten()
        .filter_map(|s| s["activity"].as_str())
        .collect();
    let mut seen = HashSet::new();
    let fresh: Vec<&Workout> = workouts
        .iter()
        .filter(|w| !linked.contains(w.id.as_str()) && seen.insert(w.id.as_str()))
        .collect();

    let mut taken = vec![false; slots.len()];
    let mut matched: Vec<(usize, &Workout)> = Vec::new();
    let mut left = Vec::new();
    for w in fresh {
        let same_day = slots.iter().enumerate().position(|(i, s)| {
            !taken[i]
                && s.day == w.day
                && w.sport.fits(&s.kind)
                && match s.status.as_str() {
                    "planned" | "missed" => true,
                    "done" => !s.linked,
                    _ => false,
                }
        });
        match same_day {
            Some(i) => {
                taken[i] = true;
                matched.push((i, w));
            }
            None => left.push(w),
        }
    }
    for w in left {
        if slots
            .iter()
            .any(|s| s.day == w.day && w.sport.fits(&s.kind))
        {
            continue;
        }
        let near = slots
            .iter()
            .enumerate()
            .filter(|(i, s)| {
                let apart = (s.day - w.day).num_days().abs();
                !taken[*i]
                    && !s.linked
                    && (1..=NEAR_DAYS).contains(&apart)
                    && w.sport.fits(&s.kind)
                    && s.kind != "race"
                    && matches!(s.status.as_str(), "planned" | "missed")
            })
            .min_by_key(|(_, s)| ((s.day - w.day).num_days().abs(), s.day))
            .map(|(i, _)| i);
        if let Some(i) = near {
            taken[i] = true;
            matched.push((i, w));
        }
    }

    let mut plan = content.clone();
    let mut done = Vec::with_capacity(matched.len());
    for (i, w) in matched {
        let slot = &slots[i];
        let session = &mut plan["weeks"][slot.week]["sessions"][slot.n];
        let before = session.clone();
        session["status"] = json!("done");
        session["activity"] = json!(w.id);
        session["actual"] = actual(w, slot.day);
        done.push(Done {
            session: before,
            workout: w.clone(),
        });
    }
    let mut missed = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        if !taken[i]
            && slot.status == "planned"
            && slot.kind != "rest"
            && slot.day + Duration::days(GRACE_DAYS) < today
        {
            let session = &mut plan["weeks"][slot.week]["sessions"][slot.n];
            missed.push(session.clone());
            session["status"] = json!("missed");
        }
    }
    if done.is_empty() && missed.is_empty() {
        return None;
    }
    done.sort_by_key(|d| d.workout.day);
    Some(Tracked {
        content: plan,
        done,
        missed,
    })
}

/// What a workout came to, as a session keeps it: only the measures a
/// plan would accept, and the day it was run when that was another.
fn actual(w: &Workout, session_day: NaiveDate) -> Value {
    let mut out = json!({});
    if let Some(km) = w.distance_km.filter(|k| *k > 0.0 && *k <= MAX_SESSION_KM) {
        out["distance_km"] = json!((km * 100.0).round() / 100.0);
    }
    if let Some(min) = w.duration_min.filter(|m| *m > 0.0 && *m <= MAX_MINUTES) {
        out["duration_min"] = json!((min * 100.0).round() / 100.0);
    }
    if let Some(p) = w.pace.as_deref().and_then(pace) {
        out["pace"] = json!(p);
    }
    if w.day != session_day {
        out["date"] = json!(w.day.to_string());
    }
    out
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "training plan",
        reason: reason.into(),
    }
}

fn sessions(content: &Value) -> impl Iterator<Item = &Value> {
    content["weeks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["sessions"].as_array())
        .flatten()
}

fn day_of(s: &Value) -> Option<NaiveDate> {
    s["date"].as_str().and_then(parse_date)
}

/// Whether `after`, a model's new version of the plan `before`, leaves
/// alone what has happened by `today`, and why not when it does not:
///
/// - the race day;
/// - every session before today, as it was and in its order;
/// - every session from today on that is not planned any more: run,
///   missed, skipped or moved, by Niles, by the person or by an earlier
///   version;
/// - and it marks no session done, missed or skipped itself. It may
///   move one (`moved`, with the new one added), since that is planning.
///
/// Both are as their kind keeps them, so equal is equal.
pub fn keeps_the_past(before: &Value, after: &Value, today: NaiveDate) -> Result<()> {
    if before["goal"]["date"] != after["goal"]["date"] {
        return Err(invalid(format!(
            "the race day stays {}: only the person changes it, by asking",
            before["goal"]["date"].as_str().unwrap_or("as it is")
        )));
    }
    let past = |c: &'_ Value| -> Vec<Value> {
        sessions(c)
            .filter(|s| day_of(s).is_some_and(|d| d < today))
            .cloned()
            .collect()
    };
    let (was, now) = (past(before), past(after));
    if was != now {
        let at = was
            .iter()
            .zip(&now)
            .position(|(a, b)| a != b)
            .unwrap_or(was.len().min(now.len()));
        let which = was
            .get(at)
            .or_else(|| now.get(at))
            .map(session_line)
            .unwrap_or_default();
        return Err(invalid(format!(
            "the sessions before today ({today}) stay exactly as they are, and {which} does not: \
             read the plan again with artifact_get and change only the ones from today on that \
             are still planned"
        )));
    }
    let settled: Vec<&Value> = sessions(before)
        .filter(|s| day_of(s).is_some_and(|d| d >= today) && s["status"] != "planned")
        .collect();
    let kept: Vec<&Value> = sessions(after).collect();
    if let Some(gone) = settled.iter().find(|s| !kept.contains(*s)) {
        return Err(invalid(format!(
            "{} is {} and stays as it is: read the plan again with artifact_get and change only \
             the sessions still planned",
            session_line(gone),
            gone["status"].as_str().unwrap_or("settled")
        )));
    }
    if let Some(marked) = sessions(after).find(|s| {
        day_of(s).is_some_and(|d| d >= today)
            && !matches!(s["status"].as_str(), Some("planned" | "moved"))
            && !settled.contains(s)
    }) {
        return Err(invalid(format!(
            "{} is marked {}: a session is done when a run comes in, and skipped when the person \
             says so, so leave the ones to come planned (or moved, with the new one added)",
            session_line(marked),
            marked["status"].as_str().unwrap_or_default()
        )));
    }
    Ok(())
}

/// How far a week is planned to go and has been run, in km: a done
/// session counts what was run when that is known. Rest days and
/// sessions without a distance count nothing.
pub fn week_km(week: &Value) -> (f64, f64) {
    let mut planned = 0.0;
    let mut run = 0.0;
    for s in week["sessions"].as_array().into_iter().flatten() {
        let km = s["distance_km"].as_f64().unwrap_or(0.0);
        planned += km;
        if s["status"] == "done" {
            run += s["actual"]["distance_km"].as_f64().unwrap_or(km);
        }
    }
    let round = |x: f64| (x * 10.0).round() / 10.0;
    (round(planned), round(run))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds;

    fn day(s: &str) -> NaiveDate {
        parse_date(s).unwrap()
    }

    /// A week from Monday 12 October: easy Tuesday, intervals Thursday,
    /// strength Friday, rest Saturday, long Sunday; and the race the
    /// Sunday after.
    fn plan() -> Value {
        kinds::validate(
            "training_plan",
            &json!({
                "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": "2026-10-25" },
                "weeks": [
                    { "starts": "2026-10-12", "sessions": [
                        { "date": "2026-10-13", "type": "easy", "title": "Easy run", "distance_km": 6 },
                        { "date": "2026-10-15", "type": "intervals", "title": "6 × 400 m", "duration_min": 40 },
                        { "date": "2026-10-16", "type": "strength", "title": "Core" },
                        { "date": "2026-10-17", "type": "rest", "title": "Rest" },
                        { "date": "2026-10-18", "type": "long", "title": "Long run", "distance_km": 10 }
                    ]},
                    { "starts": "2026-10-19", "sessions": [
                        { "date": "2026-10-20", "type": "easy", "title": "Easy run", "distance_km": 5 },
                        { "date": "2026-10-25", "type": "race", "title": "Race day", "distance_km": 10 }
                    ]}
                ]
            }),
        )
        .unwrap()
    }

    fn run(id: &str, on: &str, km: f64) -> Workout {
        Workout {
            id: id.into(),
            day: day(on),
            sport: Sport::Run,
            distance_km: Some(km),
            duration_min: Some(km * 6.3),
            pace: Some("6:18".into()),
        }
    }

    fn session<'a>(content: &'a Value, on: &str) -> &'a Value {
        sessions(content)
            .find(|s| s["date"] == on)
            .unwrap_or_else(|| panic!("a session on {on}"))
    }

    #[test]
    fn a_run_on_its_day_is_that_session_done_with_what_was_run() {
        let t = track(
            &plan(),
            &[run("111", "2026-10-13", 6.12)],
            day("2026-10-13"),
        )
        .unwrap();
        let s = session(&t.content, "2026-10-13");
        assert_eq!(s["status"], "done");
        assert_eq!(s["activity"], "111");
        assert_eq!(
            s["actual"],
            json!({ "distance_km": 6.12, "duration_min": 38.56, "pace": "6:18" })
        );
        assert_eq!(t.done.len(), 1);
        assert_eq!(t.done[0].session["status"], "planned");
        assert!(t.missed.is_empty());
        assert_eq!(t.reason(), "Done: Easy run on Tue 13 Oct");
        kinds::validate("training_plan", &t.content).unwrap();
    }

    #[test]
    fn the_same_run_again_changes_nothing() {
        let today = day("2026-10-13");
        let once = track(&plan(), &[run("111", "2026-10-13", 6.0)], today).unwrap();
        assert_eq!(
            track(&once.content, &[run("111", "2026-10-13", 6.0)], today),
            None
        );
        let twice = track(
            &plan(),
            &[run("111", "2026-10-13", 6.0), run("111", "2026-10-13", 6.0)],
            today,
        )
        .unwrap();
        assert_eq!(twice.done.len(), 1);
    }

    #[test]
    fn a_run_a_day_late_is_the_session_it_was_with_the_day_it_was_run() {
        let t = track(&plan(), &[run("222", "2026-10-14", 6.0)], day("2026-10-14")).unwrap();
        let s = session(&t.content, "2026-10-13");
        assert_eq!(s["status"], "done");
        assert_eq!(s["actual"]["date"], "2026-10-14");
        kinds::validate("training_plan", &t.content).unwrap();
    }

    #[test]
    fn of_two_sessions_as_near_the_earlier_is_the_one_run_late() {
        // Wednesday is a day after Tuesday's easy run and a day before
        // Thursday's intervals.
        let t = track(&plan(), &[run("333", "2026-10-14", 6.0)], day("2026-10-14")).unwrap();
        assert_eq!(session(&t.content, "2026-10-13")["status"], "done");
        assert_eq!(session(&t.content, "2026-10-15")["status"], "planned");
    }

    #[test]
    fn a_second_run_on_a_day_with_its_own_session_is_an_extra_one() {
        let today = day("2026-10-13");
        let t = track(
            &plan(),
            &[run("1", "2026-10-13", 6.0), run("2", "2026-10-13", 3.0)],
            today,
        )
        .unwrap();
        assert_eq!(t.done.len(), 1, "Thursday's intervals are still to come");
        assert_eq!(session(&t.content, "2026-10-15")["status"], "planned");
    }

    #[test]
    fn a_run_further_off_than_two_days_or_near_the_race_is_no_session() {
        // Friday the 23rd: three days after Tuesday's easy run, two
        // before the race.
        let t = track(&plan(), &[run("4", "2026-10-23", 5.0)], day("2026-10-23")).unwrap();
        assert!(t.done.is_empty());
        assert_eq!(session(&t.content, "2026-10-25")["status"], "planned");
        assert!(!t.content.to_string().contains("\"activity\""));
    }

    #[test]
    fn only_a_workout_of_its_kind_is_a_session() {
        let ride = Workout {
            sport: Sport::Cross,
            ..run("5", "2026-10-13", 20.0)
        };
        assert_eq!(track(&plan(), &[ride], day("2026-10-13")), None);
        let lifting = Workout {
            sport: Sport::Strength,
            distance_km: None,
            pace: None,
            ..run("6", "2026-10-16", 0.0)
        };
        let t = track(&plan(), &[lifting], day("2026-10-16")).unwrap();
        assert_eq!(session(&t.content, "2026-10-16")["status"], "done");
        assert_eq!(
            session(&t.content, "2026-10-16")["actual"]["duration_min"],
            json!(null)
        );
        // Nor is a run a rest day.
        let t = track(&plan(), &[run("7", "2026-10-17", 5.0)], day("2026-10-17")).unwrap();
        assert_eq!(session(&t.content, "2026-10-17")["status"], "planned");
        assert_eq!(session(&t.content, "2026-10-18")["status"], "done");
    }

    #[test]
    fn a_day_is_missed_only_once_the_day_after_it_is_over_and_never_a_rest_day() {
        assert_eq!(track(&plan(), &[], day("2026-10-14")), None);
        let t = track(&plan(), &[], day("2026-10-15")).unwrap();
        assert_eq!(session(&t.content, "2026-10-13")["status"], "missed");
        assert_eq!(t.reason(), "Missed: Easy run on Tue 13 Oct");

        let t = track(&plan(), &[], day("2026-10-20")).unwrap();
        let missed: Vec<&str> = t
            .missed
            .iter()
            .map(|s| s["date"].as_str().unwrap())
            .collect();
        assert_eq!(
            missed,
            ["2026-10-13", "2026-10-15", "2026-10-16", "2026-10-18"]
        );
        assert_eq!(session(&t.content, "2026-10-17")["status"], "planned");
        kinds::validate("training_plan", &t.content).unwrap();
    }

    #[test]
    fn a_late_upload_turns_a_missed_session_into_a_done_one() {
        let missed = track(&plan(), &[], day("2026-10-16")).unwrap().content;
        assert_eq!(session(&missed, "2026-10-13")["status"], "missed");
        let t = track(&missed, &[run("8", "2026-10-13", 6.0)], day("2026-10-16")).unwrap();
        assert_eq!(session(&t.content, "2026-10-13")["status"], "done");
        assert!(t.missed.is_empty());
    }

    #[test]
    fn what_the_person_skipped_or_moved_is_left_alone() {
        let mut p = plan();
        p["weeks"][0]["sessions"][0]["status"] = json!("skipped");
        p["weeks"][0]["sessions"][1]["status"] = json!("moved");
        let t = track(&p, &[run("9", "2026-10-13", 6.0)], day("2026-10-20"));
        let t = t.unwrap();
        assert_eq!(session(&t.content, "2026-10-13")["status"], "skipped");
        assert_eq!(session(&t.content, "2026-10-15")["status"], "moved");
        assert!(t.done.is_empty(), "the day had its own session");
    }

    #[test]
    fn a_session_ticked_by_hand_gets_its_run_on_its_own_day() {
        let mut p = plan();
        p["weeks"][0]["sessions"][0]["status"] = json!("done");
        let t = track(&p, &[run("10", "2026-10-13", 6.4)], day("2026-10-13")).unwrap();
        let s = session(&t.content, "2026-10-13");
        assert_eq!(s["activity"], "10");
        assert_eq!(s["actual"]["distance_km"], 6.4);
    }

    #[test]
    fn measures_a_plan_would_refuse_are_left_out() {
        let ultra = Workout {
            distance_km: Some(120.0),
            duration_min: Some(900.0),
            pace: Some("45:00".into()),
            ..run("11", "2026-10-13", 0.0)
        };
        let t = track(&plan(), &[ultra], day("2026-10-13")).unwrap();
        assert_eq!(session(&t.content, "2026-10-13")["actual"], json!({}));
        kinds::validate("training_plan", &t.content).unwrap();
    }

    fn reviewed(f: impl FnOnce(&mut Value)) -> Result<()> {
        let before = track(&plan(), &[run("1", "2026-10-13", 6.0)], day("2026-10-16"))
            .unwrap()
            .content;
        let before = kinds::validate("training_plan", &before).unwrap();
        let mut after = before.clone();
        f(&mut after);
        let after = kinds::validate("training_plan", &after).unwrap();
        keeps_the_past(&before, &after, day("2026-10-16"))
    }

    #[test]
    fn a_review_may_change_what_is_still_to_come() {
        reviewed(|p| {
            p["weeks"][1]["sessions"][0]["distance_km"] = json!(4);
            p["weeks"][0]["sessions"][2]["title"] = json!("Core and hips");
            p["updates"] = json!([{ "date": "2026-10-16", "note": "Eased next week a little." }]);
        })
        .unwrap();
    }

    #[test]
    fn a_review_may_not_rewrite_what_happened() {
        let e = reviewed(|p| p["weeks"][0]["sessions"][0]["actual"]["distance_km"] = json!(8))
            .unwrap_err()
            .to_string();
        assert!(e.contains("before today (2026-10-16)"), "{e}");
        assert!(e.contains("Tue 13 Oct · Easy run"), "{e}");

        let e = reviewed(|p| {
            p["weeks"][0]["sessions"].as_array_mut().unwrap().remove(0);
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("before today"), "{e}");

        let e = reviewed(|p| p["goal"]["date"] = json!("2026-11-01"));
        assert!(e.unwrap_err().to_string().contains("race day stays"));
    }

    #[test]
    fn a_review_may_not_undo_or_mark_a_session_itself() {
        let today_done = |p: &mut Value| {
            p["weeks"][0]["sessions"][2]["status"] = json!("done");
        };
        let e = reviewed(today_done).unwrap_err().to_string();
        assert!(e.contains("is marked done"), "{e}");

        // Moving one is planning, and allowed.
        reviewed(|p| p["weeks"][0]["sessions"][2]["status"] = json!("moved")).unwrap();

        // Settled by the person since: it stays so.
        let mut before = plan();
        before["weeks"][1]["sessions"][0]["status"] = json!("skipped");
        let mut after = before.clone();
        after["weeks"][1]["sessions"][0]["status"] = json!("planned");
        let e = keeps_the_past(&before, &after, day("2026-10-16"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("is skipped and stays"), "{e}");
    }

    #[test]
    fn a_week_adds_up_what_was_planned_and_what_was_run() {
        let t = track(&plan(), &[run("1", "2026-10-13", 6.4)], day("2026-10-13")).unwrap();
        assert_eq!(week_km(&t.content["weeks"][0]), (16.0, 6.4));
        assert_eq!(week_km(&t.content["weeks"][1]), (15.0, 0.0));
    }
}

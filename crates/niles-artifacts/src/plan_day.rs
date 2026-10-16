//! A training plan's day: which of a person's plans is the one they are
//! on now, and what it has for one day of it — that day's sessions, the
//! next one to run, how far the week has gone, and the coach's newest
//! note.
//!
//! Read alike by `training_today`, which says it, and by the app's
//! dashboard card, which shows it: the same plan and the same day for
//! both, with the saying and the showing left to each. Sessions come as
//! the plan keeps them, in km and per-km paces.
//!
//! Days are the house's days, and the caller says which day today is.

use crate::error::Result;
use crate::model::{Artifact, Filter};
use crate::store::ArtifactStore;
use crate::training_plan::{KIND, parse_date};
use chrono::{Datelike, Duration, NaiveDate};
use serde_json::Value;

/// The plan that is theirs now: of those not archived whose race is
/// still to come, a pinned one first, then the one changed last — the
/// order the store lists them in. With the content of the version shown.
pub async fn current_plan(
    store: &dyn ArtifactStore,
    person: &str,
    today: NaiveDate,
) -> Result<Option<(Artifact, Value)>> {
    let filter = Filter {
        archived: Some(false),
        ..Filter::default()
    };
    for a in store.list(person, filter).await? {
        if a.kind != KIND.id || a.current_revision == 0 {
            continue;
        }
        let Some(r) = store.revision(person, a.id, a.current_revision).await? else {
            continue;
        };
        if race_day(&r.content).is_some_and(|race| race >= today) {
            return Ok(Some((a, r.content)));
        }
    }
    Ok(None)
}

fn race_day(content: &Value) -> Option<NaiveDate> {
    content["goal"]["date"].as_str().and_then(parse_date)
}

fn day_of(session: &Value) -> Option<NaiveDate> {
    session["date"].as_str().and_then(parse_date)
}

/// One day of a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanDay<'a> {
    pub day: NaiveDate,
    pub race_day: NaiveDate,
    /// From today, not from `day`: tomorrow's run is still nine days
    /// from a race nine days off.
    pub days_to_go: i64,
    /// That day's sessions, rest days among them, in the plan's order.
    pub sessions: Vec<&'a Value>,
    /// The first session after the day that is still planned and not a
    /// rest day.
    pub next: Option<&'a Value>,
    pub week: Option<WeekSoFar<'a>>,
    /// The coach's newest note, `{date, note}`.
    pub update: Option<&'a Value>,
}

/// The week the day is in, so far.
#[derive(Debug, Clone, PartialEq)]
pub struct WeekSoFar<'a> {
    pub focus: Option<&'a str>,
    pub planned_km: f64,
    pub run_km: f64,
    /// Sessions from the day on still to run, rest days not counted.
    pub sessions_left: usize,
}

impl<'a> PlanDay<'a> {
    /// `day` of the plan in `content`, as kept by its kind.
    pub fn of(content: &'a Value, day: NaiveDate, today: NaiveDate) -> Self {
        let weeks = content["weeks"].as_array().map_or(&[][..], Vec::as_slice);
        let sessions = || {
            weeks
                .iter()
                .filter_map(|w| w["sessions"].as_array())
                .flatten()
        };
        let race_day = race_day(content).unwrap_or(today);
        let week = weeks
            .iter()
            .find(|w| {
                w["starts"]
                    .as_str()
                    .and_then(parse_date)
                    .is_some_and(|starts| {
                        let sunday = starts
                            + Duration::days(i64::from(
                                6 - starts.weekday().num_days_from_monday(),
                            ));
                        starts <= day && day <= sunday
                    })
            })
            .map(|w| {
                let (planned_km, run_km) = crate::tracking::week_km(w);
                WeekSoFar {
                    focus: w["focus"].as_str(),
                    planned_km,
                    run_km,
                    sessions_left: w["sessions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|s| still_to_run(s) && day_of(s).is_some_and(|d| d >= day))
                        .count(),
                }
            });
        PlanDay {
            day,
            race_day,
            days_to_go: (race_day - today).num_days(),
            sessions: sessions().filter(|s| day_of(s) == Some(day)).collect(),
            next: sessions().find(|s| still_to_run(s) && day_of(s).is_some_and(|d| d > day)),
            week,
            update: content["updates"].as_array().and_then(|u| u.first()),
        }
    }

    /// Whether there is nothing to run that day: no session, or only
    /// rest.
    pub fn rest_day(&self) -> bool {
        self.sessions.iter().all(|s| s["type"] == "rest")
    }
}

fn still_to_run(s: &Value) -> bool {
    s["status"] == "planned" && s["type"] != "rest"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Author, Changes, NewArtifact, NewRevision, Status};
    use crate::store::MemoryArtifacts;
    use serde_json::json;

    fn on(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    /// Toward a 10K on `race`, the 25th or later: Tuesday's easy run done,
    /// Friday's intervals today, Saturday's rest, Sunday's long run.
    fn plan(race: &str) -> Value {
        crate::kinds::validate(
            KIND.id,
            &json!({
                "goal": { "race": "Hellerup 10K", "distance_km": 10, "date": race },
                "weeks": [
                    { "starts": "2026-10-12", "focus": "Build", "sessions": [
                        { "date": "2026-10-13", "type": "easy", "title": "Easy run", "distance_km": 6,
                          "status": "done", "activity": "1", "actual": { "distance_km": 6.4 } },
                        { "date": "2026-10-16", "type": "intervals", "title": "6 × 400 m",
                          "distance_km": 7, "pace": "4:45-5:00" },
                        { "date": "2026-10-17", "type": "rest", "title": "Rest" },
                        { "date": "2026-10-18", "type": "long", "title": "Long run", "distance_km": 12 }
                    ]},
                    { "starts": "2026-10-19", "sessions": [
                        { "date": "2026-10-25", "type": "race", "title": "Race day", "distance_km": 10 }
                    ]}
                ],
                "updates": [
                    { "date": "2026-10-13", "note": "Good start." },
                    { "date": "2026-10-14", "note": "Eased Friday a little." }
                ]
            }),
        )
        .unwrap()
    }

    async fn made(store: &MemoryArtifacts, person: &str, kind: &str, content: Value) -> i64 {
        store
            .create(
                person,
                NewArtifact {
                    kind: kind.into(),
                    title: "A plan".into(),
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
                        summary: None,
                        page: crate::PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
            .id
    }

    #[test]
    fn a_day_has_its_sessions_the_next_one_and_its_week() {
        let p = plan("2026-10-25");
        let d = PlanDay::of(&p, on(16), on(16));
        assert_eq!(d.sessions.len(), 1);
        assert_eq!(d.sessions[0]["title"], "6 × 400 m");
        assert!(!d.rest_day());
        assert_eq!(d.next.unwrap()["title"], "Long run", "past the rest day");
        assert_eq!(d.days_to_go, 9);
        assert_eq!(d.race_day, on(25));
        let week = d.week.unwrap();
        assert_eq!(week.focus, Some("Build"));
        assert_eq!((week.planned_km, week.run_km), (25.0, 6.4));
        assert_eq!(week.sessions_left, 2);
        assert_eq!(d.update.unwrap()["note"], "Eased Friday a little.");
    }

    #[test]
    fn a_rest_day_or_an_empty_one_has_nothing_to_run() {
        let p = plan("2026-10-25");
        assert!(PlanDay::of(&p, on(17), on(16)).rest_day());
        let empty = PlanDay::of(&p, on(14), on(14));
        assert!(empty.sessions.is_empty());
        assert!(empty.rest_day());
    }

    #[test]
    fn the_countdown_is_from_today_whatever_day_is_read() {
        let p = plan("2026-10-25");
        let tomorrow = PlanDay::of(&p, on(17), on(16));
        assert_eq!(tomorrow.days_to_go, 9);
        assert_eq!(tomorrow.week.unwrap().sessions_left, 1);
    }

    #[tokio::test]
    async fn the_current_plan_is_hers_still_to_run_and_pinned_first() {
        let store = MemoryArtifacts::new();
        let her = "person:majse";
        let pinned = made(&store, her, KIND.id, plan("2026-10-25")).await;
        let newest = made(&store, her, KIND.id, plan("2026-10-25")).await;
        let run = json!({
            "goal": { "race": "Autumn 5K", "distance_km": 5, "date": "2026-10-11" },
            "weeks": [{ "starts": "2026-10-05", "sessions": [] }]
        });
        made(&store, her, KIND.id, run).await;
        made(&store, her, "document", json!({ "markdown": "# notes" })).await;
        made(&store, "person:mark", KIND.id, plan("2026-10-25")).await;

        let (a, _) = current_plan(&store, her, on(16)).await.unwrap().unwrap();
        assert_eq!(a.id, newest, "the one changed last whose race is to come");
        store
            .change(
                her,
                pinned,
                Changes {
                    pinned: Some(true),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        let (a, content) = current_plan(&store, her, on(16)).await.unwrap().unwrap();
        assert_eq!(a.id, pinned);
        assert_eq!(content["goal"]["date"], "2026-10-25");

        assert!(current_plan(&store, her, on(26)).await.unwrap().is_none());
        assert!(
            current_plan(&store, "person:guest", on(16))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_archived_plan_is_not_the_current_one() {
        let store = MemoryArtifacts::new();
        let her = "person:majse";
        let id = made(&store, her, KIND.id, plan("2026-10-25")).await;
        store
            .change(
                her,
                id,
                Changes {
                    archived: Some(true),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        assert!(current_plan(&store, her, on(16)).await.unwrap().is_none());
    }
}

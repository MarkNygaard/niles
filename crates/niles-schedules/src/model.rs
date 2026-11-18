//! A schedule, when it runs, and what became of each run.

use crate::error::{Error, Result};
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use niles_automations::Trigger;
use serde::{Deserialize, Serialize};

/// The longest name a schedule may have.
pub const MAX_NAME: usize = 60;
/// The longest instruction a schedule may have.
pub const MAX_INSTRUCTION: usize = 2000;

/// When a schedule runs, in the household's time zone. Never more often
/// than once an hour: every run is the assistant's time and tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    /// Every day at `at` (`HH:MM`).
    Daily { at: String },
    /// Monday to Friday at `at`.
    Weekdays { at: String },
    /// On the days named, at `at`.
    Days { days: Vec<Weekday>, at: String },
    /// Every `hours` hours from midnight, on the hour: 1, 2, 3, 4, 6, 8
    /// or 12 divide the day evenly.
    EveryHours { hours: u8 },
}

fn invalid(kind: &'static str, reason: &str) -> Error {
    Error::Invalid {
        kind,
        reason: reason.to_string(),
    }
}

fn clock(at: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(at.trim(), "%H:%M")
        .map_err(|_| invalid("time", "it must be a time of day such as 08:00"))
}

impl When {
    /// Checked, with its time written the one way it is kept.
    pub fn checked(self) -> Result<When> {
        let canonical = |at: &str| clock(at).map(|t| t.format("%H:%M").to_string());
        Ok(match self {
            When::Daily { at } => When::Daily {
                at: canonical(&at)?,
            },
            When::Weekdays { at } => When::Weekdays {
                at: canonical(&at)?,
            },
            When::Days { mut days, at } => {
                days.sort_by_key(Weekday::num_days_from_monday);
                days.dedup();
                if days.is_empty() {
                    return Err(invalid("days", "choose at least one day"));
                }
                When::Days {
                    days,
                    at: canonical(&at)?,
                }
            }
            When::EveryHours { hours } => {
                if ![1, 2, 3, 4, 6, 8, 12].contains(&hours) {
                    return Err(invalid(
                        "hours",
                        "every 1, 2, 3, 4, 6, 8 or 12 hours: once an hour at most, evenly over the day",
                    ));
                }
                When::EveryHours { hours }
            }
        })
    }

    /// The automations' clock triggers it is: one per time of day.
    pub fn triggers(&self) -> Vec<Trigger> {
        let time = |at: &str| clock(at).unwrap_or(NaiveTime::MIN);
        match self {
            When::Daily { at } => vec![Trigger::Time {
                at: time(at),
                days: None,
            }],
            When::Weekdays { at } => vec![Trigger::Time {
                at: time(at),
                days: Some(vec![
                    Weekday::Mon,
                    Weekday::Tue,
                    Weekday::Wed,
                    Weekday::Thu,
                    Weekday::Fri,
                ]),
            }],
            When::Days { days, at } => vec![Trigger::Time {
                at: time(at),
                days: Some(days.clone()),
            }],
            When::EveryHours { hours } => (0..24u32)
                .step_by(usize::from((*hours).max(1)))
                .filter_map(|h| NaiveTime::from_hms_opt(h, 0, 0))
                .map(|at| Trigger::Time { at, days: None })
                .collect(),
        }
    }

    /// The first time after `after` it runs, as the automations' clock
    /// triggers decide it day by day in `tz`. A time a daylight-saving
    /// change skips that day doesn't run that day, as an automation's
    /// doesn't.
    pub fn next_after(&self, after: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
        let triggers = self.triggers();
        let first = after.with_timezone(&tz).date_naive();
        (0..=8)
            .filter_map(|offset| first.checked_add_signed(Duration::days(offset)))
            .flat_map(|day| {
                // Midday of that local day: the date the triggers read.
                let noon = day
                    .and_hms_opt(12, 0, 0)
                    .and_then(|n| tz.from_local_datetime(&n).earliest());
                let triggers = &triggers;
                noon.into_iter().flat_map(move |noon| {
                    triggers.iter().filter_map(move |t| {
                        let local = t.due_on(noon.with_timezone(&Utc), tz, None)?;
                        tz.from_local_datetime(&local).earliest()
                    })
                })
            })
            .map(|at| at.with_timezone(&Utc))
            .filter(|at| *at > after)
            .min()
    }
}

/// A name a person gave, checked.
pub fn name(given: &str) -> Result<String> {
    let given = given.trim();
    if given.is_empty() {
        return Err(invalid("name", "it is empty"));
    }
    if given.chars().count() > MAX_NAME || given.chars().any(char::is_control) {
        return Err(invalid("name", "60 characters at most, on one line"));
    }
    Ok(given.to_string())
}

/// An instruction a person wrote, checked.
pub fn instruction(given: &str) -> Result<String> {
    let given = given.trim();
    if given.is_empty() {
        return Err(invalid("instruction", "say what Niles should do"));
    }
    if given.chars().count() > MAX_INSTRUCTION {
        return Err(invalid("instruction", "2000 characters at most"));
    }
    Ok(given.to_string())
}

/// A schedule as it is added or changed, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSchedule {
    pub name: String,
    pub instruction: String,
    pub when: When,
    /// The saved login whose sites its runs may act on without asking:
    /// clicks, choices, submits and signing in with that login
    /// (NIL-87). Checked to be its person's own before it is kept.
    pub acts_on: Option<i64>,
}

impl NewSchedule {
    pub fn new(given_name: &str, given_instruction: &str, when: When) -> Result<NewSchedule> {
        Ok(NewSchedule {
            name: name(given_name)?,
            instruction: instruction(given_instruction)?,
            when: when.checked()?,
            acts_on: None,
        })
    }

    /// The same schedule, allowed to act on its own on `login`'s sites.
    #[must_use]
    pub fn acting_on(mut self, login: Option<i64>) -> NewSchedule {
        self.acts_on = login;
        self
    }
}

/// A person's schedule, as My profile lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Schedule {
    pub id: i64,
    pub name: String,
    pub instruction: String,
    pub when: When,
    pub paused: bool,
    /// When it runs next; `None` while paused.
    pub next_due: Option<DateTime<Utc>>,
    /// The saved login whose sites its runs may act on without asking.
    pub acts_on: Option<i64>,
}

/// What became of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    /// The assistant answered.
    Done,
    /// The answer waits on a card in its conversation.
    Waiting,
    Failed,
    /// Niles wasn't running when it was due; not caught up.
    Missed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Running => "running",
            Outcome::Done => "done",
            Outcome::Waiting => "waiting",
            Outcome::Failed => "failed",
            Outcome::Missed => "missed",
        }
    }

    pub fn parse(name: &str) -> Outcome {
        match name {
            "running" => Outcome::Running,
            "done" => Outcome::Done,
            "waiting" => Outcome::Waiting,
            "missed" => Outcome::Missed,
            _ => Outcome::Failed,
        }
    }
}

/// One run of a schedule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub id: i64,
    /// When it was due.
    pub due: DateTime<Utc>,
    pub finished: Option<DateTime<Utc>>,
    pub outcome: Outcome,
    /// The conversation its answer is in.
    pub conversation: Option<i64>,
    /// Why it failed, in Niles's words.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn copenhagen() -> Tz {
        "Europe/Copenhagen".parse().unwrap()
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        copenhagen()
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_daily_schedule_runs_at_its_local_time_next() {
        let when = When::Daily { at: "08:00".into() }.checked().unwrap();
        // 2026-11-16 is a Monday.
        assert_eq!(
            when.next_after(local(2026, 11, 16, 7, 0), copenhagen()),
            Some(local(2026, 11, 16, 8, 0))
        );
        assert_eq!(
            when.next_after(local(2026, 11, 16, 8, 0), copenhagen()),
            Some(local(2026, 11, 17, 8, 0)),
            "not twice at the same minute"
        );
    }

    #[test]
    fn weekdays_skip_the_weekend_and_days_keep_to_theirs() {
        let weekdays = When::Weekdays { at: "07:30".into() }.checked().unwrap();
        // Friday evening: next is Monday.
        assert_eq!(
            weekdays.next_after(local(2026, 11, 20, 19, 0), copenhagen()),
            Some(local(2026, 11, 23, 7, 30))
        );
        let mondays = When::Days {
            days: vec![Weekday::Mon],
            at: "8:00".into(),
        }
        .checked()
        .unwrap();
        assert_eq!(
            mondays,
            When::Days {
                days: vec![Weekday::Mon],
                at: "08:00".into()
            }
        );
        assert_eq!(
            mondays.next_after(local(2026, 11, 16, 9, 0), copenhagen()),
            Some(local(2026, 11, 23, 8, 0))
        );
    }

    #[test]
    fn every_few_hours_is_on_the_hour_from_midnight() {
        let when = When::EveryHours { hours: 6 }.checked().unwrap();
        assert_eq!(
            when.next_after(local(2026, 11, 16, 7, 15), copenhagen()),
            Some(local(2026, 11, 16, 12, 0))
        );
        assert_eq!(
            when.next_after(local(2026, 11, 16, 19, 0), copenhagen()),
            Some(local(2026, 11, 17, 0, 0))
        );
    }

    #[test]
    fn the_local_time_holds_across_daylight_saving() {
        let when = When::Daily { at: "08:00".into() }.checked().unwrap();
        // Summer time ends on 2026-10-25 in Copenhagen: 08:00 local is
        // 06:00 UTC before, 07:00 UTC after.
        let before = when
            .next_after(local(2026, 10, 24, 9, 0), copenhagen())
            .unwrap();
        assert_eq!(before, Utc.with_ymd_and_hms(2026, 10, 25, 7, 0, 0).unwrap());
    }

    #[test]
    fn more_often_than_hourly_or_unevenly_is_refused() {
        for hours in [0, 5, 7, 24, 48] {
            assert!(When::EveryHours { hours }.checked().is_err(), "{hours}");
        }
        assert!(
            When::Days {
                days: vec![],
                at: "08:00".into()
            }
            .checked()
            .is_err()
        );
        assert!(
            When::Daily {
                at: "8 o'clock".into()
            }
            .checked()
            .is_err()
        );
        assert!(NewSchedule::new("", "x", When::Daily { at: "08:00".into() }).is_err());
        assert!(NewSchedule::new("x", " ", When::Daily { at: "08:00".into() }).is_err());
    }

    #[test]
    fn a_schedule_reads_back_as_it_was_written() {
        let when = When::Days {
            days: vec![Weekday::Wed, Weekday::Mon],
            at: "18:05".into(),
        }
        .checked()
        .unwrap();
        let json = serde_json::to_string(&when).unwrap();
        assert_eq!(serde_json::from_str::<When>(&json).unwrap(), when);
    }
}

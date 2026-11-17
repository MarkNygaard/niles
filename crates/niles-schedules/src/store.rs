//! Where each person's schedules and their runs are kept: Postgres in
//! the house (niles-db), memory in tests.
//!
//! What a person reaches is scoped by their key, so another person's id
//! answers like one never kept. `due`, `set_next` and `finish_run` are
//! the runner's, never a route's.

use crate::error::Result;
use crate::model::{NewSchedule, Outcome, Run, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

/// How many of a schedule's latest runs are shown.
pub const RUNS_SHOWN: usize = 50;

#[async_trait]
pub trait ScheduleStore: Send + Sync {
    /// The person's own, by name then id.
    async fn list(&self, person: &str) -> Result<Vec<Schedule>>;
    async fn get(&self, person: &str, id: i64) -> Result<Option<Schedule>>;
    async fn add(
        &self,
        person: &str,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Schedule>;
    /// `None` when the id isn't the person's.
    async fn update(
        &self,
        person: &str,
        id: i64,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Option<Schedule>>;
    async fn remove(&self, person: &str, id: i64) -> Result<bool>;
    /// Paused has no next run; resumed, `next` is when it runs again.
    async fn set_paused(
        &self,
        person: &str,
        id: i64,
        paused: bool,
        next: Option<DateTime<Utc>>,
    ) -> Result<bool>;
    /// Every person's schedules not paused and due by `now`.
    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<(String, Schedule)>>;
    /// When the schedule runs next, after a run or a miss.
    async fn set_next(&self, id: i64, next: Option<DateTime<Utc>>) -> Result<()>;
    /// A run begun (or missed) for the person's schedule; its id.
    async fn start_run(
        &self,
        person: &str,
        id: i64,
        due: DateTime<Utc>,
        outcome: Outcome,
    ) -> Result<Option<i64>>;
    async fn finish_run(
        &self,
        run: i64,
        outcome: Outcome,
        conversation: Option<i64>,
        reason: Option<&str>,
    ) -> Result<()>;
    /// The schedule's latest runs, newest first; `None` when the id
    /// isn't the person's.
    async fn runs(&self, person: &str, id: i64) -> Result<Option<Vec<Run>>>;
    /// Every run still marked running, failed for `reason`: Niles
    /// stopped while they ran, and nothing will finish them.
    async fn abandon_running(&self, reason: &str) -> Result<()>;
}

struct Row {
    person: String,
    schedule: Schedule,
    runs: Vec<Run>,
}

#[derive(Default)]
struct Kept {
    rows: Vec<Row>,
    next_id: i64,
    next_run: i64,
}

/// The in-memory store, for tests.
#[derive(Default)]
pub struct MemorySchedules {
    kept: Mutex<Kept>,
}

impl MemorySchedules {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl ScheduleStore for MemorySchedules {
    async fn list(&self, person: &str) -> Result<Vec<Schedule>> {
        let kept = self.lock();
        let mut out: Vec<Schedule> = kept
            .rows
            .iter()
            .filter(|r| r.person == person)
            .map(|r| r.schedule.clone())
            .collect();
        out.sort_by_key(|s| (s.name.to_lowercase(), s.id));
        Ok(out)
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Schedule>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .find(|r| r.person == person && r.schedule.id == id)
            .map(|r| r.schedule.clone()))
    }

    async fn add(
        &self,
        person: &str,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Schedule> {
        let mut kept = self.lock();
        kept.next_id += 1;
        let schedule = Schedule {
            id: kept.next_id,
            name: new.name.clone(),
            instruction: new.instruction.clone(),
            when: new.when.clone(),
            paused: false,
            next_due: next,
        };
        kept.rows.push(Row {
            person: person.to_string(),
            schedule: schedule.clone(),
            runs: Vec::new(),
        });
        Ok(schedule)
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Option<Schedule>> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.schedule.id == id)
        else {
            return Ok(None);
        };
        row.schedule.name = new.name.clone();
        row.schedule.instruction = new.instruction.clone();
        row.schedule.when = new.when.clone();
        if !row.schedule.paused {
            row.schedule.next_due = next;
        }
        Ok(Some(row.schedule.clone()))
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        let mut kept = self.lock();
        let before = kept.rows.len();
        kept.rows
            .retain(|r| !(r.person == person && r.schedule.id == id));
        Ok(kept.rows.len() != before)
    }

    async fn set_paused(
        &self,
        person: &str,
        id: i64,
        paused: bool,
        next: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let mut kept = self.lock();
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.schedule.id == id)
        else {
            return Ok(false);
        };
        row.schedule.paused = paused;
        row.schedule.next_due = if paused { None } else { next };
        Ok(true)
    }

    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<(String, Schedule)>> {
        Ok(self
            .lock()
            .rows
            .iter()
            .filter(|r| !r.schedule.paused && r.schedule.next_due.is_some_and(|at| at <= now))
            .map(|r| (r.person.clone(), r.schedule.clone()))
            .collect())
    }

    async fn set_next(&self, id: i64, next: Option<DateTime<Utc>>) -> Result<()> {
        if let Some(row) = self.lock().rows.iter_mut().find(|r| r.schedule.id == id) {
            row.schedule.next_due = next;
        }
        Ok(())
    }

    async fn start_run(
        &self,
        person: &str,
        id: i64,
        due: DateTime<Utc>,
        outcome: Outcome,
    ) -> Result<Option<i64>> {
        let mut kept = self.lock();
        kept.next_run += 1;
        let run_id = kept.next_run;
        let Some(row) = kept
            .rows
            .iter_mut()
            .find(|r| r.person == person && r.schedule.id == id)
        else {
            return Ok(None);
        };
        row.runs.push(Run {
            id: run_id,
            due,
            finished: (outcome != Outcome::Running).then(Utc::now),
            outcome,
            conversation: None,
            reason: None,
        });
        Ok(Some(run_id))
    }

    async fn finish_run(
        &self,
        run: i64,
        outcome: Outcome,
        conversation: Option<i64>,
        reason: Option<&str>,
    ) -> Result<()> {
        let mut kept = self.lock();
        if let Some(found) = kept
            .rows
            .iter_mut()
            .flat_map(|r| r.runs.iter_mut())
            .find(|r| r.id == run)
        {
            found.outcome = outcome;
            found.finished = Some(Utc::now());
            found.conversation = conversation;
            found.reason = reason.map(str::to_string);
        }
        Ok(())
    }

    async fn runs(&self, person: &str, id: i64) -> Result<Option<Vec<Run>>> {
        let kept = self.lock();
        let Some(row) = kept
            .rows
            .iter()
            .find(|r| r.person == person && r.schedule.id == id)
        else {
            return Ok(None);
        };
        let mut runs = row.runs.clone();
        runs.reverse();
        runs.truncate(RUNS_SHOWN);
        Ok(Some(runs))
    }

    async fn abandon_running(&self, reason: &str) -> Result<()> {
        let mut kept = self.lock();
        let running = kept
            .rows
            .iter_mut()
            .flat_map(|r| r.runs.iter_mut())
            .filter(|r| r.outcome == Outcome::Running);
        for run in running {
            run.outcome = Outcome::Failed;
            run.finished = Some(Utc::now());
            run.reason = Some(reason.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::When;

    const MAJSE: &str = "person:majse";
    const MARK: &str = "person:mark";

    fn linear() -> NewSchedule {
        NewSchedule::new(
            "Linear on Monday",
            "my open Linear issues by priority",
            When::Days {
                days: vec![chrono::Weekday::Mon],
                at: "08:00".into(),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_schedule_and_its_runs_are_its_persons_alone() {
        let store = MemorySchedules::new();
        let schedule = store.add(MAJSE, &linear(), Some(Utc::now())).await.unwrap();
        assert!(store.list(MARK).await.unwrap().is_empty());
        assert!(store.get(MARK, schedule.id).await.unwrap().is_none());
        assert!(!store.remove(MARK, schedule.id).await.unwrap());
        assert!(
            !store
                .set_paused(MARK, schedule.id, true, None)
                .await
                .unwrap()
        );
        assert!(
            store
                .start_run(MARK, schedule.id, Utc::now(), Outcome::Running)
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.runs(MARK, schedule.id).await.unwrap().is_none());
        assert!(
            store
                .update(MARK, schedule.id, &linear(), None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn only_what_is_due_and_not_paused_is_due() {
        let store = MemorySchedules::new();
        let now = Utc::now();
        let due = store
            .add(MAJSE, &linear(), Some(now - chrono::Duration::minutes(1)))
            .await
            .unwrap();
        let later = store
            .add(MAJSE, &linear(), Some(now + chrono::Duration::hours(1)))
            .await
            .unwrap();
        let paused = store
            .add(MARK, &linear(), Some(now - chrono::Duration::minutes(1)))
            .await
            .unwrap();
        store.set_paused(MARK, paused.id, true, None).await.unwrap();
        let ids: Vec<i64> = store
            .due(now)
            .await
            .unwrap()
            .iter()
            .map(|(_, s)| s.id)
            .collect();
        assert_eq!(ids, [due.id]);
        assert!(!ids.contains(&later.id));
    }

    #[tokio::test]
    async fn a_run_is_kept_with_what_became_of_it() {
        let store = MemorySchedules::new();
        let schedule = store.add(MAJSE, &linear(), None).await.unwrap();
        let run = store
            .start_run(MAJSE, schedule.id, Utc::now(), Outcome::Running)
            .await
            .unwrap()
            .unwrap();
        store
            .finish_run(run, Outcome::Waiting, Some(7), None)
            .await
            .unwrap();
        let runs = store.runs(MAJSE, schedule.id).await.unwrap().unwrap();
        assert_eq!(runs[0].outcome, Outcome::Waiting);
        assert_eq!(runs[0].conversation, Some(7));
        assert!(runs[0].finished.is_some());
    }
}

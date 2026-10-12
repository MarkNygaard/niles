//! What each rule did lately: the last runs, kept in memory for as long
//! as Niles runs — including the ones a condition stopped, because "why
//! didn't the light turn off?" is the question they answer.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many runs are kept per rule.
pub const KEPT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Still going: waiting out a delay, most likely.
    Running,
    /// Every step done.
    Done,
    /// A condition did not hold; nothing was done.
    Stopped,
    /// A newer run of the same rule took its place.
    Replaced,
    /// A step went wrong.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    pub number: u64,
    pub started_at: DateTime<Utc>,
    /// The id of the trigger that started it.
    pub trigger: Option<String>,
    pub outcome: Outcome,
    /// Why it stopped or failed, in a few words.
    pub reason: Option<String>,
    /// What it did, step by step.
    pub steps: Vec<String>,
}

#[derive(Default)]
pub struct RunLog {
    runs: Mutex<HashMap<String, VecDeque<Run>>>,
    next: AtomicU64,
}

impl RunLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note a run starting — or, for one a condition stopped, the whole
    /// of it. Returns its number, for the steps that follow.
    pub fn begin(
        &self,
        rule: &str,
        trigger: Option<&str>,
        at: DateTime<Utc>,
        outcome: Outcome,
        reason: Option<String>,
    ) -> u64 {
        let number = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut runs = self.lock();
        let kept = runs.entry(rule.to_string()).or_default();
        kept.push_front(Run {
            number,
            started_at: at,
            trigger: trigger.map(String::from),
            outcome,
            reason,
            steps: Vec::new(),
        });
        kept.truncate(KEPT);
        number
    }

    pub fn step(&self, rule: &str, number: u64, done: String) {
        self.with(rule, number, |run| run.steps.push(done));
    }

    pub fn finish(&self, rule: &str, number: u64, outcome: Outcome, reason: Option<String>) {
        self.with(rule, number, |run| {
            run.outcome = outcome;
            run.reason = reason;
        });
    }

    /// A run another took the place of — if it was still going; one
    /// that had already ended keeps how it ended.
    pub fn replaced(&self, rule: &str, number: u64) {
        self.with(rule, number, |run| {
            if run.outcome == Outcome::Running {
                run.outcome = Outcome::Replaced;
            }
        });
    }

    /// A rule's runs, newest first.
    pub fn runs(&self, rule: &str) -> Vec<Run> {
        self.lock()
            .get(rule)
            .map(|r| r.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn with(&self, rule: &str, number: u64, change: impl FnOnce(&mut Run)) {
        if let Some(run) = self
            .lock()
            .get_mut(rule)
            .and_then(|r| r.iter_mut().find(|run| run.number == number))
        {
            change(run);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, VecDeque<Run>>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_twenty_with_their_steps() {
        let log = RunLog::new();
        let mut last = 0;
        for _ in 0..25 {
            last = log.begin("r", Some("t1"), Utc::now(), Outcome::Running, None);
        }
        log.step("r", last, "set the office lights off".into());
        log.finish("r", last, Outcome::Done, None);
        let runs = log.runs("r");
        assert_eq!(runs.len(), KEPT);
        assert_eq!(runs[0].number, last);
        assert_eq!(runs[0].outcome, Outcome::Done);
        assert_eq!(runs[0].steps, ["set the office lights off"]);
        assert!(log.runs("other").is_empty());
    }
}

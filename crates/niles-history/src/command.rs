//! Command history: each spoken turn, what was said and answered.

use crate::error::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Mutex;

/// One voice-command turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandEntry {
    /// When the turn was heard.
    pub ts: DateTime<Utc>,
    /// Satellite peer that sent the audio.
    pub peer: SocketAddr,
    /// Canonical room of the satellite, if known.
    #[serde(default)]
    pub origin_room: Option<String>,
    /// Transcribed user utterance.
    pub transcript: String,
    /// Spoken response returned to the user (None = no response / error).
    #[serde(default)]
    pub spoken_response: Option<String>,
    /// Identified speaker display name, if recognition ran and matched.
    #[serde(default)]
    pub speaker: Option<String>,
}

/// Filter for [`CommandHistory::query`].
#[derive(Debug, Clone, Default)]
pub struct CommandQuery {
    /// Inclusive lower bound on `ts`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `ts`.
    pub until: Option<DateTime<Utc>>,
    /// Match `origin_room` exactly.
    pub room: Option<String>,
    /// Max rows to return (default 50, clamped to 500).
    pub limit: Option<usize>,
}

impl CommandQuery {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(50).min(500)
    }

    pub fn check(&self) -> Result<()> {
        crate::check_range(self.since, self.until)
    }

    fn matches(&self, e: &CommandEntry) -> bool {
        !(self.since.is_some_and(|since| e.ts < since)
            || self.until.is_some_and(|until| e.ts > until)
            || self
                .room
                .as_ref()
                .is_some_and(|room| e.origin_room.as_ref() != Some(room)))
    }
}

#[async_trait]
pub trait CommandHistory: Send + Sync {
    async fn record(&self, entry: &CommandEntry) -> Result<()>;

    /// Entries matching `filter`, newest first.
    async fn query(&self, filter: &CommandQuery) -> Result<Vec<CommandEntry>>;

    /// When the oldest entry kept is from. `None` when nothing is kept.
    async fn earliest(&self) -> Result<Option<DateTime<Utc>>>;

    async fn forget_before(&self, before: DateTime<Utc>) -> Result<()>;
}

/// Kept as long as the process runs: for tests.
#[derive(Default)]
pub struct MemoryCommandHistory {
    entries: Mutex<Vec<CommandEntry>>,
}

impl MemoryCommandHistory {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<CommandEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl CommandHistory for MemoryCommandHistory {
    async fn record(&self, entry: &CommandEntry) -> Result<()> {
        self.entries().push(entry.clone());
        Ok(())
    }

    async fn query(&self, filter: &CommandQuery) -> Result<Vec<CommandEntry>> {
        filter.check()?;
        let mut found: Vec<CommandEntry> = self
            .entries()
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();
        found.sort_by_key(|e| std::cmp::Reverse(e.ts));
        found.truncate(filter.limit());
        Ok(found)
    }

    async fn earliest(&self) -> Result<Option<DateTime<Utc>>> {
        Ok(self.entries().iter().map(|e| e.ts).min())
    }

    async fn forget_before(&self, before: DateTime<Utc>) -> Result<()> {
        self.entries().retain(|e| e.ts >= before);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 30, hour, 0, 0).unwrap()
    }

    fn said(ts: DateTime<Utc>, room: Option<&str>, transcript: &str) -> CommandEntry {
        CommandEntry {
            ts,
            peer: "192.168.1.50:5000".parse().unwrap(),
            origin_room: room.map(str::to_string),
            transcript: transcript.into(),
            spoken_response: Some("Done.".into()),
            speaker: None,
        }
    }

    async fn kept(entries: &[CommandEntry]) -> MemoryCommandHistory {
        let history = MemoryCommandHistory::new();
        for e in entries {
            history.record(e).await.unwrap();
        }
        history
    }

    #[tokio::test]
    async fn a_query_is_newest_first_in_its_room_and_hours() {
        let history = kept(&[
            said(at(18), Some("kitchen"), "lights on"),
            said(at(19), Some("living_room"), "dim the lights"),
            said(at(20), Some("living_room"), "movie time"),
            said(at(23), Some("living_room"), "lights off"),
        ])
        .await;
        let found = history
            .query(&CommandQuery {
                since: Some(at(18)),
                until: Some(at(21)),
                room: Some("living_room".into()),
                limit: None,
            })
            .await
            .unwrap();
        let said: Vec<_> = found.iter().map(|e| e.transcript.as_str()).collect();
        assert_eq!(said, ["movie time", "dim the lights"]);
    }

    #[tokio::test]
    async fn a_turn_without_a_room_is_left_out_of_a_room_s_query() {
        let history = kept(&[said(at(19), None, "what time is it")]).await;
        let found = history
            .query(&CommandQuery {
                room: Some("kitchen".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn a_range_that_ends_before_it_starts_is_refused() {
        let backwards = CommandQuery {
            since: Some(at(21)),
            until: Some(at(20)),
            ..Default::default()
        };
        assert!(MemoryCommandHistory::new().query(&backwards).await.is_err());
    }

    #[tokio::test]
    async fn what_is_older_than_kept_goes() {
        let history = kept(&[said(at(18), None, "a"), said(at(20), None, "b")]).await;
        history.forget_before(at(19)).await.unwrap();
        assert_eq!(history.earliest().await.unwrap(), Some(at(20)));
    }
}

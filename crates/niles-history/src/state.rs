//! Device state history: each device's whole state after each change.

use crate::error::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_core::{DeviceId, DeviceState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// One device's state at one moment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateEntry {
    /// When the state changed.
    pub ts: DateTime<Utc>,
    /// Source-qualified device identifier.
    pub device_id: DeviceId,
    /// The device's whole state from then on, not only what changed:
    /// the newest entry before a moment is how the device was then.
    pub state: DeviceState,
}

/// Filter for [`StateHistory::query`].
#[derive(Debug, Clone, Default)]
pub struct StateQuery {
    /// Inclusive lower bound on `ts`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `ts`.
    pub until: Option<DateTime<Utc>>,
    /// Match a specific device id exactly.
    pub device_id: Option<DeviceId>,
    /// Match devices in this room (`device_id` wins if both are set).
    pub room: Option<String>,
    /// Max rows to return (default 200, clamped to 2000).
    pub limit: Option<usize>,
}

impl StateQuery {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(200).min(2000)
    }

    pub fn check(&self) -> Result<()> {
        crate::check_range(self.since, self.until)
    }

    fn matches(&self, e: &StateEntry) -> bool {
        if self.since.is_some_and(|since| e.ts < since)
            || self.until.is_some_and(|until| e.ts > until)
        {
            return false;
        }
        match (&self.device_id, &self.room) {
            (Some(id), _) => e.device_id == *id,
            (None, Some(room)) => e.device_id.room().as_str() == room,
            (None, None) => true,
        }
    }
}

#[async_trait]
pub trait StateHistory: Send + Sync {
    async fn record(&self, entry: &StateEntry) -> Result<()>;

    /// Entries matching `filter`, newest first.
    async fn query(&self, filter: &StateQuery) -> Result<Vec<StateEntry>>;

    /// Each of `ids`' newest entry at or before `at`, ordered by id.
    /// An id with none is left out.
    async fn snapshot_at(&self, at: DateTime<Utc>, ids: &[DeviceId]) -> Result<Vec<StateEntry>>;

    /// When the oldest entry kept is from: what an empty answer is
    /// measured against. `None` when nothing is kept.
    async fn earliest(&self) -> Result<Option<DateTime<Utc>>>;

    async fn forget_before(&self, before: DateTime<Utc>) -> Result<()>;
}

/// Kept as long as the process runs: for tests.
#[derive(Default)]
pub struct MemoryStateHistory {
    entries: Mutex<Vec<StateEntry>>,
}

impl MemoryStateHistory {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<StateEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl StateHistory for MemoryStateHistory {
    async fn record(&self, entry: &StateEntry) -> Result<()> {
        self.entries().push(entry.clone());
        Ok(())
    }

    async fn query(&self, filter: &StateQuery) -> Result<Vec<StateEntry>> {
        filter.check()?;
        let mut found: Vec<StateEntry> = self
            .entries()
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();
        found.sort_by_key(|e| std::cmp::Reverse(e.ts));
        found.truncate(filter.limit());
        Ok(found)
    }

    async fn snapshot_at(&self, at: DateTime<Utc>, ids: &[DeviceId]) -> Result<Vec<StateEntry>> {
        let mut latest: HashMap<&DeviceId, StateEntry> = HashMap::new();
        let entries = self.entries();
        for e in entries.iter().filter(|e| e.ts <= at) {
            let Some(id) = ids.iter().find(|id| **id == e.device_id) else {
                continue;
            };
            if latest.get(id).is_none_or(|kept| kept.ts <= e.ts) {
                latest.insert(id, e.clone());
            }
        }
        let mut found: Vec<StateEntry> = latest.into_values().collect();
        found.sort_by_key(|e| e.device_id.to_string());
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

    fn id(s: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{s}")).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 30, hour, minute, 0).unwrap()
    }

    fn entry(ts: DateTime<Utc>, device: &str, brightness: u8) -> StateEntry {
        StateEntry {
            ts,
            device_id: id(device),
            state: DeviceState {
                on: Some(true),
                brightness: Some(brightness),
                ..Default::default()
            },
        }
    }

    async fn kept(entries: &[StateEntry]) -> MemoryStateHistory {
        let history = MemoryStateHistory::new();
        for e in entries {
            history.record(e).await.unwrap();
        }
        history
    }

    #[tokio::test]
    async fn a_query_is_newest_first_and_limited() {
        let history = kept(&[
            entry(at(19, 0), "living_room/table", 40),
            entry(at(20, 0), "living_room/table", 30),
            entry(at(21, 0), "living_room/table", 20),
        ])
        .await;
        let found = history
            .query(&StateQuery {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        let brightness: Vec<_> = found.iter().map(|e| e.state.brightness).collect();
        assert_eq!(brightness, [Some(20), Some(30)]);
    }

    #[tokio::test]
    async fn a_query_keeps_to_its_room_and_its_hours() {
        let history = kept(&[
            entry(at(19, 0), "living_room/table", 40),
            entry(at(20, 0), "kitchen/ceiling", 30),
            entry(at(22, 0), "living_room/table", 20),
        ])
        .await;
        let found = history
            .query(&StateQuery {
                since: Some(at(18, 0)),
                until: Some(at(21, 0)),
                room: Some("living_room".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(found, [entry(at(19, 0), "living_room/table", 40)]);
    }

    #[tokio::test]
    async fn a_range_that_ends_before_it_starts_is_refused() {
        let history = MemoryStateHistory::new();
        let backwards = StateQuery {
            since: Some(at(21, 0)),
            until: Some(at(20, 0)),
            ..Default::default()
        };
        assert!(history.query(&backwards).await.is_err());
    }

    #[tokio::test]
    async fn a_snapshot_is_each_device_as_it_was_then() {
        let history = kept(&[
            entry(at(19, 0), "living_room/table", 40),
            entry(at(19, 55), "living_room/table", 30),
            entry(at(20, 30), "living_room/table", 100),
            entry(at(18, 0), "living_room/floor_lamp", 10),
            entry(at(19, 0), "kitchen/ceiling", 70),
        ])
        .await;
        let ids = [id("living_room/table"), id("living_room/floor_lamp")];
        let then = history.snapshot_at(at(20, 0), &ids).await.unwrap();
        assert_eq!(
            then,
            [
                entry(at(18, 0), "living_room/floor_lamp", 10),
                entry(at(19, 55), "living_room/table", 30),
            ]
        );
    }

    #[tokio::test]
    async fn a_snapshot_leaves_out_a_device_with_nothing_kept_before_then() {
        let history = kept(&[entry(at(21, 0), "living_room/table", 40)]).await;
        let then = history
            .snapshot_at(at(20, 0), &[id("living_room/table")])
            .await
            .unwrap();
        assert!(then.is_empty());
    }

    #[tokio::test]
    async fn the_oldest_is_known_and_what_is_older_than_kept_goes() {
        let history = kept(&[
            entry(at(18, 0), "living_room/table", 40),
            entry(at(20, 0), "living_room/table", 30),
        ])
        .await;
        assert_eq!(history.earliest().await.unwrap(), Some(at(18, 0)));
        history.forget_before(at(19, 0)).await.unwrap();
        assert_eq!(history.earliest().await.unwrap(), Some(at(20, 0)));
        assert_eq!(MemoryStateHistory::new().earliest().await.unwrap(), None);
    }
}

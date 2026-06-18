//! In-memory timer store for voice-dispatch timers.
//!
//! Timers live in a `HashMap<TimerId, TimerEntry>` behind an `RwLock`
//! so they can be queried from both sync and async contexts.
//! Optional file persistence (JSON) survives process restarts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use crate::persistence::{atomic_write_json, read_json_or_empty};

mod serde_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        let ms: u64 = d
            .as_millis()
            .try_into()
            .map_err(|_| serde::ser::Error::custom("duration exceeds u64 millis"))?;
        s.serialize_u64(ms)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Opaque identifier for a timer entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimerId(pub u64);

/// State machine for a single timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerState {
    /// Waiting for the driver loop to reach `expires_at`.
    Pending,
    /// The driver has woken on this timer and fired the event.
    /// Remains in the store until explicitly stopped or cancelled.
    Ringing,
}

/// A single timer as stored in the [`TimerStore`].
#[derive(Debug, Clone)]
pub struct TimerEntry {
    pub id: TimerId,
    /// Canonical (trim + lowercase + underscore) name, or `None`
    /// if the user didn't name the timer.
    pub name: Option<String>,
    /// Original duration the user asked for.
    pub duration: Duration,
    /// Absolute wall-clock expiry time.
    pub expires_at: DateTime<Utc>,
    /// The satellite (peer) that issued the original `TimerSet`.
    pub origin: SocketAddr,
    /// Current lifecycle state.
    pub state: TimerState,
}

impl TimerEntry {
    /// Returns `true` while the timer is waiting to fire.
    pub fn is_pending(&self) -> bool {
        self.state == TimerState::Pending
    }

    /// Returns `true` once the driver has transitioned the timer to
    /// [`TimerState::Ringing`].
    pub fn is_ringing(&self) -> bool {
        self.state == TimerState::Ringing
    }
}

/// Internal mutable state of the store.
#[derive(Debug, Default)]
struct TimerStoreInner {
    next_id: u64,
    timers: HashMap<TimerId, TimerEntry>,
}

// ------------------------------------------------------------------
// Persistence DTOs
// ------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct PersistedTimer {
    id: TimerId,
    name: Option<String>,
    #[serde(with = "serde_duration")]
    duration: Duration,
    expires_at: DateTime<Utc>,
    origin: SocketAddr,
    state: TimerState,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedTimers {
    entries: Vec<PersistedTimer>,
}

impl From<&TimerEntry> for PersistedTimer {
    fn from(e: &TimerEntry) -> Self {
        Self {
            id: e.id,
            name: e.name.clone(),
            duration: e.duration,
            expires_at: e.expires_at,
            origin: e.origin,
            state: e.state,
        }
    }
}

impl From<PersistedTimer> for TimerEntry {
    fn from(p: PersistedTimer) -> Self {
        Self {
            id: p.id,
            name: p.name,
            duration: p.duration,
            expires_at: p.expires_at,
            origin: p.origin,
            state: p.state,
        }
    }
}

/// In-memory store for pending / ringing timers.
///
/// Mirrors the `RwLock<HashMap>` shape of [`SceneStore`](crate::scenes::SceneStore)
/// and [`ManualModeTracker`](crate::manual_mode::ManualModeTracker).
pub struct TimerStore {
    inner: RwLock<TimerStoreInner>,
    persistence_path: Option<PathBuf>,
}

impl TimerStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(TimerStoreInner {
                next_id: 1,
                timers: HashMap::new(),
            }),
            persistence_path: None,
        }
    }

    /// Configure a persistence path. Call before handing the store
    /// to driver/tasks. Returns `self` for chaining.
    pub fn with_persistence(mut self, path: PathBuf) -> Self {
        self.persistence_path = Some(path);
        self
    }

    /// Load timers from a JSON file. Missing or corrupt files yield
    /// an empty store (logged, not fatal).
    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let persisted: PersistedTimers = read_json_or_empty(path, "timers")?;
        let now = Utc::now();
        let mut max_id = 0u64;
        let timers: HashMap<TimerId, TimerEntry> = persisted
            .entries
            .into_iter()
            .map(|mut p| {
                max_id = max_id.max(p.id.0);
                if p.state == TimerState::Pending && p.expires_at <= now {
                    p.state = TimerState::Ringing;
                }
                let entry: TimerEntry = p.into();
                (entry.id, entry)
            })
            .collect();
        Ok(Self {
            inner: RwLock::new(TimerStoreInner {
                next_id: max_id.wrapping_add(1),
                timers,
            }),
            persistence_path: None,
        })
    }

    /// Save timers to a JSON file (atomic via tempfile).
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let inner = self.inner_read();
        self.save_locked(&inner, path)
    }

    fn save_locked(&self, inner: &TimerStoreInner, path: &Path) -> std::io::Result<()> {
        let mut entries: Vec<PersistedTimer> =
            inner.timers.values().map(PersistedTimer::from).collect();
        entries.sort_by_key(|e| e.id);
        atomic_write_json(path, &PersistedTimers { entries })
    }

    fn maybe_save(&self, inner: &TimerStoreInner) {
        if let Some(path) = self.persistence_path.as_deref()
            && let Err(e) = self.save_locked(inner, path)
        {
            tracing::warn!("persistence: timers save failed: {e}");
        }
    }

    /// Register a new timer. Returns its assigned [`TimerId`].
    /// `name` is the raw user-said text; it is canonicalized before
    /// storage. `now` is injected so tests can use a fixed clock.
    pub fn set(
        &self,
        duration: Duration,
        name: Option<String>,
        origin: SocketAddr,
        now: DateTime<Utc>,
    ) -> TimerId {
        let mut inner = self.inner_write();
        let id = TimerId(next_vacant_id(&mut inner));
        // Voice input can produce absurd durations ("set a timer for
        // a trillion minutes"). `chrono::Duration::from_std` rejects
        // values beyond i64-milliseconds, and `DateTime + Duration`
        // panics on year-range overflow. Treat both as "fire now"
        // rather than crash the driver task.
        let expires_at = chrono::Duration::from_std(duration)
            .ok()
            .and_then(|d| now.checked_add_signed(d))
            .unwrap_or(now);
        let entry = TimerEntry {
            id,
            name: name.as_deref().map(canonicalize_name),
            duration,
            expires_at,
            origin,
            state: TimerState::Pending,
        };
        inner.timers.insert(id, entry);
        self.maybe_save(&inner);
        id
    }

    /// Remove a timer by exact id. Returns `true` if it was present.
    pub fn cancel(&self, id: TimerId) -> bool {
        let mut inner = self.inner_write();
        let removed = inner.timers.remove(&id).is_some();
        self.maybe_save(&inner);
        removed
    }

    /// Remove every timer whose canonical name matches `name`.
    /// Returns the number of timers removed.
    pub fn cancel_by_name(&self, name: &str) -> usize {
        let key = canonicalize_name(name);
        let mut inner = self.inner_write();
        let before = inner.timers.len();
        inner.timers.retain(|_, e| e.name.as_ref() != Some(&key));
        let removed = before - inner.timers.len();
        self.maybe_save(&inner);
        removed
    }

    /// Find the most recently-expired *ringing* timer, remove it,
    /// and return the entry. Returns `None` if no timer is ringing.
    pub fn stop_most_recent_ringing(&self) -> Option<TimerEntry> {
        let mut inner = self.inner_write();
        let id = inner
            .timers
            .values()
            .filter(|e| e.is_ringing())
            .max_by_key(|e| e.expires_at)
            .map(|e| e.id)?;
        let entry = inner.timers.remove(&id);
        self.maybe_save(&inner);
        entry
    }

    /// Remove the soonest-expiring *pending* timer and return it.
    /// Returns `None` if no timer is pending. Used by a generic
    /// "stop"/"cancel" to abort a counting-down timer when none is
    /// ringing yet.
    pub fn cancel_soonest_pending(&self) -> Option<TimerEntry> {
        let mut inner = self.inner_write();
        let id = inner
            .timers
            .values()
            .filter(|e| e.is_pending())
            .min_by_key(|e| e.expires_at)
            .map(|e| e.id)?;
        let entry = inner.timers.remove(&id);
        self.maybe_save(&inner);
        entry
    }

    /// Transition `Pending → Ringing` for `id`. Returns the updated
    /// entry on the first call; returns `None` if the timer is already
    /// ringing or absent.
    pub fn mark_ringing(&self, id: TimerId) -> Option<TimerEntry> {
        let mut inner = self.inner_write();
        let entry = inner.timers.get_mut(&id)?;
        if entry.state != TimerState::Pending {
            return None;
        }
        entry.state = TimerState::Ringing;
        let cloned = entry.clone();
        self.maybe_save(&inner);
        Some(cloned)
    }

    /// Return all timers sorted by `expires_at` (soonest first).
    pub fn list(&self) -> Vec<TimerEntry> {
        let mut entries: Vec<TimerEntry> = self.inner_read().timers.values().cloned().collect();
        entries.sort_by_key(|e| e.expires_at);
        entries
    }

    /// Soonest `expires_at` among `Pending` timers, or `None` if
    /// there are no pending timers.
    pub fn next_expiry(&self) -> Option<DateTime<Utc>> {
        self.inner_read()
            .timers
            .values()
            .filter(|e| e.is_pending())
            .map(|e| e.expires_at)
            .min()
    }

    // ---- lock helpers -----------------------------------------------------
    //
    // `std::sync::RwLock` poisons on a panic in any thread holding
    // the write lock. Our operations are trivial (HashMap inserts
    // and removes) and don't panic in practice, but defensively
    // recovering from poison via `into_inner()` means a panic in
    // one path can't cascade-kill the whole store. The recovered
    // state is consistent because each individual lock guards a
    // single collection that's mutated atomically inside the
    // critical section.

    fn inner_write(&self) -> std::sync::RwLockWriteGuard<'_, TimerStoreInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    fn inner_read(&self) -> std::sync::RwLockReadGuard<'_, TimerStoreInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for TimerStore {
    fn default() -> Self {
        Self::new()
    }
}

fn next_vacant_id(inner: &mut TimerStoreInner) -> u64 {
    let mut candidate = if inner.next_id == 0 { 1 } else { inner.next_id };
    let start = candidate;
    loop {
        let id = TimerId(candidate);
        if !inner.timers.contains_key(&id) {
            inner.next_id = candidate.wrapping_add(1);
            if inner.next_id == 0 {
                inner.next_id = 1;
            }
            return candidate;
        }
        candidate = candidate.wrapping_add(1);
        if candidate == 0 {
            candidate = 1;
        }
        if candidate == start {
            // Practically unreachable (would require 2^64-1 live timers), but
            // avoid an infinite loop if ID space is exhausted.
            panic!("timer id space exhausted");
        }
    }
}

/// Normalize a raw timer name for use as a HashMap key.
///
/// Rules: trim, lowercase ASCII, collapse runs of ASCII whitespace to `_`.
pub fn canonicalize_name(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn localhost() -> SocketAddr {
        "127.0.0.1:9999".parse().unwrap()
    }

    #[test]
    fn set_returns_monotonic_ids() {
        let store = TimerStore::new();
        let now = Utc::now();
        assert_eq!(
            store.set(Duration::from_secs(60), None, localhost(), now),
            TimerId(1)
        );
        assert_eq!(
            store.set(Duration::from_secs(120), None, localhost(), now),
            TimerId(2)
        );
        assert_eq!(
            store.set(Duration::from_secs(180), None, localhost(), now),
            TimerId(3)
        );
    }

    #[test]
    fn set_then_list_roundtrip() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id = store.set(
            Duration::from_secs(300),
            Some("pasta".into()),
            localhost(),
            now,
        );
        let entries = store.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].name, Some("pasta".into()));
        assert_eq!(entries[0].expires_at, now + chrono::Duration::seconds(300));
    }

    #[test]
    fn cancel_present_returns_true() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);
        assert!(store.cancel(id));
        assert!(store.list().is_empty());
    }

    #[test]
    fn cancel_absent_returns_false() {
        let store = TimerStore::new();
        assert!(!store.cancel(TimerId(99)));
    }

    #[test]
    fn cancel_by_name_canonicalizes_case_and_whitespace() {
        let store = TimerStore::new();
        let now = Utc::now();
        store.set(
            Duration::from_secs(60),
            Some("Pasta".into()),
            localhost(),
            now,
        );
        assert_eq!(store.cancel_by_name(" PASTA "), 1);
        assert_eq!(store.cancel_by_name(" PASTA "), 0);
    }

    #[test]
    fn cancel_by_name_unknown_returns_zero() {
        let store = TimerStore::new();
        assert_eq!(store.cancel_by_name("nope"), 0);
    }

    #[test]
    fn mark_ringing_transitions_once_then_none() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);
        let first = store.mark_ringing(id);
        assert!(first.is_some());
        assert!(first.unwrap().is_ringing());
        assert!(store.mark_ringing(id).is_none());
    }

    #[test]
    fn stop_most_recent_ringing_returns_latest_expires_at() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id1 = store.set(Duration::from_secs(60), None, localhost(), now);
        let id2 = store.set(Duration::from_secs(120), None, localhost(), now);
        let id3 = store.set(Duration::from_secs(180), None, localhost(), now);
        store.mark_ringing(id1);
        store.mark_ringing(id2);
        store.mark_ringing(id3);
        let stopped = store.stop_most_recent_ringing().unwrap();
        assert_eq!(stopped.id, id3);
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn stop_most_recent_ringing_none_when_empty_or_only_pending() {
        let store = TimerStore::new();
        let now = Utc::now();
        store.set(Duration::from_secs(60), None, localhost(), now);
        assert!(store.stop_most_recent_ringing().is_none());
        assert!(store.stop_most_recent_ringing().is_none());
    }

    #[test]
    fn cancel_soonest_pending_removes_earliest_pending() {
        let store = TimerStore::new();
        let now = Utc::now();
        let soonest = store.set(Duration::from_secs(60), None, localhost(), now);
        let _later = store.set(Duration::from_secs(300), None, localhost(), now);
        let cancelled = store.cancel_soonest_pending().expect("a pending timer");
        assert_eq!(cancelled.id, soonest);
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn cancel_soonest_pending_none_when_no_pending() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);
        store.mark_ringing(id); // only a ringing timer remains
        assert!(store.cancel_soonest_pending().is_none());
        assert_eq!(store.list().len(), 1, "ringing timer is left in place");
    }

    #[test]
    fn next_expiry_returns_soonest_pending_ignores_ringing() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id1 = store.set(Duration::from_secs(60), None, localhost(), now);
        let _id2 = store.set(Duration::from_secs(120), None, localhost(), now);
        store.mark_ringing(id1);
        assert_eq!(
            store.next_expiry(),
            Some(now + chrono::Duration::seconds(120))
        );
    }

    #[test]
    fn list_sorted_by_expires_at() {
        let store = TimerStore::new();
        let now = Utc::now();
        let id2 = store.set(Duration::from_secs(120), None, localhost(), now);
        let id1 = store.set(Duration::from_secs(60), None, localhost(), now);
        let id3 = store.set(Duration::from_secs(180), None, localhost(), now);
        let ids: Vec<TimerId> = store.list().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![id1, id2, id3]);
    }

    #[test]
    fn arc_clones_share_writes() {
        let store = Arc::new(TimerStore::new());
        let store2 = Arc::clone(&store);
        let now = Utc::now();
        store.set(Duration::from_secs(60), None, localhost(), now);
        assert_eq!(store2.list().len(), 1);
    }

    #[test]
    fn cancel_by_name_removes_multiple_matches() {
        let store = TimerStore::new();
        let now = Utc::now();
        store.set(
            Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        store.set(
            Duration::from_secs(120),
            Some("pasta".into()),
            localhost(),
            now,
        );
        assert_eq!(store.cancel_by_name("pasta"), 2);
        assert!(store.list().is_empty());
    }

    #[test]
    fn next_expiry_none_when_empty() {
        let store = TimerStore::new();
        assert!(store.next_expiry().is_none());
    }

    #[test]
    fn set_with_overflowing_duration_falls_back_to_now() {
        // Regression: `now + chrono::Duration::from_std(d)` panics on
        // DateTime year-range overflow. A trillion-minute timer is
        // a plausible voice-transcription artifact and must not crash
        // the driver task.
        let store = TimerStore::new();
        let now = Utc::now();
        let huge = Duration::from_secs(u64::MAX);
        let id = store.set(huge, None, localhost(), now);
        let entries = store.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].expires_at, now);
    }

    // ------------------------------------------------------------------
    // Persistence tests
    // ------------------------------------------------------------------

    #[test]
    fn persists_and_reloads_pending_timers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        let id1 = store.set(
            Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        let id2 = store.set(Duration::from_secs(120), None, localhost(), now);

        let reloaded = TimerStore::load_from_file(&path)
            .unwrap()
            .with_persistence(path);
        let entries = reloaded.list();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, id1);
        assert_eq!(entries[0].name, Some("pasta".into()));
        assert_eq!(entries[1].id, id2);
        assert_eq!(entries[1].name, None);
    }

    #[test]
    fn load_from_missing_file_yields_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let store = TimerStore::load_from_file(&path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn load_from_corrupt_file_yields_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        std::fs::write(&path, b"not json").unwrap();
        let store = TimerStore::load_from_file(&path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn loaded_pending_expired_timer_becomes_ringing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new();
        let now = Utc::now();
        let id = store.set(
            Duration::from_secs(60),
            Some("old".into()),
            localhost(),
            now,
        );
        store.save_to_file(&path).unwrap();

        // Manually rewrite expires_at to the past.
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["entries"][0]["expires_at"] =
            serde_json::json!((now - chrono::Duration::days(1)).to_rfc3339());
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

        let reloaded = TimerStore::load_from_file(&path).unwrap();
        let entries = reloaded.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].state, TimerState::Ringing);
    }

    #[test]
    fn next_id_after_reload_is_strictly_greater_than_persisted_max() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        store.set(Duration::from_secs(60), None, localhost(), now);
        store.set(Duration::from_secs(60), None, localhost(), now);
        store.set(Duration::from_secs(60), None, localhost(), now);

        let reloaded = TimerStore::load_from_file(&path)
            .unwrap()
            .with_persistence(path);
        let next_id = reloaded.set(Duration::from_secs(60), None, localhost(), now);
        assert_eq!(next_id, TimerId(4));
    }

    #[test]
    fn write_through_persists_on_set_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);

        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["entries"].as_array().unwrap().len(), 1);

        store.cancel(id);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn write_through_persists_on_cancel_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        store.set(
            Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );

        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["entries"].as_array().unwrap().len(), 1);

        store.cancel_by_name("pasta");
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn write_through_persists_on_mark_ringing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);

        let raw_before: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw_before["entries"][0]["state"], "Pending");

        store.mark_ringing(id);
        let raw_after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw_after["entries"][0]["state"], "Ringing");
    }

    #[test]
    fn write_through_persists_on_stop_most_recent_ringing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        let id = store.set(Duration::from_secs(60), None, localhost(), now);
        store.mark_ringing(id);

        let raw_before: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw_before["entries"].as_array().unwrap().len(), 1);

        store.stop_most_recent_ringing();
        let raw_after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw_after["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn overflowing_duration_fails_to_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let store = TimerStore::new().with_persistence(path.clone());
        let now = Utc::now();
        let huge = Duration::from_secs(u64::MAX);
        store.set(huge, None, localhost(), now);
        // Serialization of u64::MAX seconds exceeds u64 millis and should fail.
        // The temp file is dropped, so the target path should never be created.
        assert!(!path.exists());
    }

    #[test]
    fn reload_with_u64_max_id_allocates_non_colliding_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timers.json");
        let raw = serde_json::json!({
            "entries": [{
                "id": u64::MAX,
                "name": null,
                "duration": 1000u64,
                "expires_at": "2026-01-01T00:00:00Z",
                "origin": "127.0.0.1:9999",
                "state": "Pending"
            }]
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

        let store = TimerStore::load_from_file(&path).unwrap();
        let new_id = store.set(Duration::from_secs(60), None, localhost(), Utc::now());
        assert_eq!(new_id, TimerId(1));
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn wrapped_next_id_skips_existing_timer_ids() {
        let store = TimerStore {
            inner: RwLock::new(TimerStoreInner {
                next_id: 0,
                timers: HashMap::from([(
                    TimerId(1),
                    TimerEntry {
                        id: TimerId(1),
                        name: None,
                        duration: Duration::from_secs(60),
                        expires_at: Utc::now(),
                        origin: localhost(),
                        state: TimerState::Pending,
                    },
                )]),
            }),
            persistence_path: None,
        };
        let new_id = store.set(Duration::from_secs(60), None, localhost(), Utc::now());
        assert_eq!(new_id, TimerId(2));
        assert_eq!(store.list().len(), 2);
    }
}

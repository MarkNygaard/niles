//! In-memory timer store for voice-dispatch timers.
//!
//! v0.1: no persistence, no satellite alarm playback. Timers live
//! in a `HashMap<TimerId, TimerEntry>` behind an `RwLock` so they
//! can be queried from both sync and async contexts.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::RwLock;
use std::time::Duration;

/// Opaque identifier for a timer entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

/// State machine for a single timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// In-memory store for pending / ringing timers.
///
/// Mirrors the `RwLock<HashMap>` shape of [`SceneStore`](crate::scenes::SceneStore)
/// and [`ManualModeTracker`](crate::manual_mode::ManualModeTracker).
pub struct TimerStore {
    inner: RwLock<TimerStoreInner>,
}

impl TimerStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(TimerStoreInner {
                next_id: 1,
                timers: HashMap::new(),
            }),
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
        let id = TimerId(inner.next_id);
        inner.next_id += 1;
        let expires_at =
            now + chrono::Duration::from_std(duration).unwrap_or(chrono::Duration::zero());
        let entry = TimerEntry {
            id,
            name: name.as_deref().map(canonicalize_name),
            duration,
            expires_at,
            origin,
            state: TimerState::Pending,
        };
        inner.timers.insert(id, entry);
        id
    }

    /// Remove a timer by exact id. Returns `true` if it was present.
    pub fn cancel(&self, id: TimerId) -> bool {
        self.inner_write().timers.remove(&id).is_some()
    }

    /// Remove every timer whose canonical name matches `name`.
    /// Returns the number of timers removed.
    pub fn cancel_by_name(&self, name: &str) -> usize {
        let key = canonicalize_name(name);
        let mut inner = self.inner_write();
        let before = inner.timers.len();
        inner.timers.retain(|_, e| e.name.as_ref() != Some(&key));
        before - inner.timers.len()
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
        inner.timers.remove(&id)
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
        Some(entry.clone())
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

/// Normalize a raw timer name for use as a HashMap key.
///
/// Rules: trim, lowercase ASCII, collapse runs of ASCII whitespace to `_`.
///
/// Intentionally duplicated from `scenes::canonicalize_name` — per
/// CLAUDE.md "no premature abstractions". Promote to a shared
/// `niles-scheduler::names` module the third time it's needed.
fn canonicalize_name(raw: &str) -> String {
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
}

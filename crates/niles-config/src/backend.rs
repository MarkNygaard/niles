//! Where the override document and its history are kept.
//!
//! [`ConfigStore`](crate::ConfigStore) owns the *meaning* of an override —
//! merging it over the base, validating the result, refusing a write that
//! would produce an invalid config. Where the bytes live is a separate
//! question, and this trait is the seam.
//!
//! [`MemoryBackend`] ships here; it keeps everything in RAM and forgets
//! it on drop. The Postgres backend lives in its own crate so this one
//! keeps no database dependency.
//!
//! # Why the methods are async
//!
//! A database needs async, and the store persists *before* it swaps the
//! running config — so that a write which reaches memory but not durable
//! storage can't silently revert on the next restart. Preserving that ordering across a network call is what
//! makes the whole write path async.

use crate::Revision;
use async_trait::async_trait;

/// Everything a backend has to hold: the current override document and
/// the history that makes undo possible.
///
/// Loaded and saved together because they have to agree — a history whose
/// newest entry doesn't match the document it claims to precede would
/// undo into a state that never existed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredState {
    pub overrides: toml::Table,
    pub revisions: Vec<Revision>,
}

/// Durable storage for the override document.
///
/// Errors are reported as strings rather than a shared error type: a
/// backend's failures are its own (a filesystem error, a connection
/// refused), and the store only ever reports them as context.
#[async_trait]
pub trait OverrideBackend: Send + Sync {
    /// Load what was stored, or `None` if nothing has been written yet.
    ///
    /// A backend that can't be reached should return `Err`. The store
    /// treats that as "no overrides, and say so loudly" rather than a
    /// fatal error — the base config is still valid, and refusing to
    /// start over a tuning value would take the house down with it.
    async fn load(&self) -> Result<Option<StoredState>, String>;

    /// Persist the new state, replacing whatever was there.
    ///
    /// Must be atomic from a reader's point of view: a crash partway
    /// through cannot leave something that fails to parse on next boot.
    async fn save(&self, state: &StoredState) -> Result<(), String>;

    /// Whether writes outlive the process. False for [`MemoryBackend`],
    /// which lets callers say so rather than letting a user discover it
    /// at the next restart.
    fn is_persistent(&self) -> bool {
        true
    }

    /// Short description for logs — "the niles database". Used when
    /// reporting that a backend failed.
    fn describe(&self) -> String;
}

/// Keeps state in memory only. Writes apply and are lost on restart.
///
/// Used by tests, and as the fallback when no durable backend is
/// configured — which is a real deployment state, not just a test one.
#[derive(Default)]
pub struct MemoryBackend {
    state: std::sync::Mutex<Option<StoredState>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl OverrideBackend for MemoryBackend {
    async fn load(&self) -> Result<Option<StoredState>, String> {
        Ok(self.lock().clone())
    }

    async fn save(&self, state: &StoredState) -> Result<(), String> {
        *self.lock() = Some(state.clone());
        Ok(())
    }

    fn is_persistent(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        "memory (changes are lost on restart)".into()
    }
}

impl MemoryBackend {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<StoredState>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> StoredState {
        StoredState {
            overrides: toml::from_str("[lighting]\ndaytime_brightness = 85").unwrap(),
            revisions: vec![Revision {
                id: 1,
                at: chrono::Utc::now(),
                source: crate::ChangeSource::Voice,
                summary: "lighting.daytime_brightness 100 → 85".into(),
                changed_paths: vec!["lighting.daytime_brightness".into()],
                overrides_before: toml::Table::new(),
            }],
        }
    }

    #[tokio::test]
    async fn memory_backend_round_trips_and_admits_it_is_not_durable() {
        let backend = MemoryBackend::new();
        assert_eq!(backend.load().await.unwrap(), None);
        // One value, not two calls to `state()` — each would stamp its own
        // `Utc::now()` and the comparison would fail on microseconds.
        let saved = state();
        backend.save(&saved).await.unwrap();
        assert_eq!(backend.load().await.unwrap(), Some(saved));
        assert!(!backend.is_persistent());
    }
}

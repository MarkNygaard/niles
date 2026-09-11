//! Where the override document and its history are kept.
//!
//! [`ConfigStore`](crate::ConfigStore) owns the *meaning* of an override —
//! merging it over the base, validating the result, refusing a write that
//! would produce an invalid config. Where the bytes live is a separate
//! question, and this trait is the seam.
//!
//! Two implementations ship here: [`FileBackend`], which writes TOML into
//! a directory, and [`MemoryBackend`], which keeps everything in RAM and
//! forgets it on drop. A Postgres backend lives in its own crate so this
//! one keeps no database dependency.
//!
//! # Why the methods are async
//!
//! A local file needs no async at all. A database does, and the store
//! persists *before* it swaps the running config — so that a write which
//! reaches memory but not durable storage can't silently revert on the
//! next restart. Preserving that ordering across a network call is what
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

    /// Short description for logs — "the directory /var/lib/niles/config",
    /// "the niles database". Used when reporting that a backend failed.
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

/// TOML files in a directory: `overrides.toml` beside `revisions.toml`.
///
/// Human-readable and diffable on purpose — the same reasoning as the
/// memory store's markdown files. Someone debugging at 2am should be able
/// to `cat` the file and see what the house thinks it was told.
pub struct FileBackend {
    dir: std::path::PathBuf,
}

/// The file name of the override document inside the backend's directory.
pub(crate) const OVERRIDES_FILE: &str = "overrides.toml";

/// Companion journal, alongside the override document.
pub(crate) const REVISIONS_FILE: &str = "revisions.toml";

impl FileBackend {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

/// The journal is stored as an array of tables so it stays readable and
/// diffable by hand, like everything else in the config directory.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RevisionFile {
    #[serde(default, rename = "revision")]
    revisions: Vec<Revision>,
}

#[async_trait]
impl OverrideBackend for FileBackend {
    async fn load(&self) -> Result<Option<StoredState>, String> {
        let overrides = match std::fs::read_to_string(self.dir.join(OVERRIDES_FILE)) {
            Ok(raw) => toml::from_str(&raw).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        // A damaged journal costs history, not config: the override
        // document alone decides what is in force.
        let revisions = match std::fs::read_to_string(self.dir.join(REVISIONS_FILE)) {
            Ok(raw) => toml::from_str::<RevisionFile>(&raw)
                .map(|f| f.revisions)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Ok(Some(StoredState {
            overrides,
            revisions,
        }))
    }

    async fn save(&self, state: &StoredState) -> Result<(), String> {
        let overrides = toml::to_string_pretty(&state.overrides).map_err(|e| e.to_string())?;
        let journal = toml::to_string_pretty(&RevisionFile {
            revisions: state.revisions.clone(),
        })
        .map_err(|e| e.to_string())?;
        write_atomically(&self.dir, OVERRIDES_FILE, &overrides)?;
        write_atomically(&self.dir, REVISIONS_FILE, &journal)
    }

    fn describe(&self) -> String {
        format!("the directory {}", self.dir.display())
    }
}

/// Temp file + rename, so a crash mid-write can't leave a half-parsed
/// file that fails the next boot.
fn write_atomically(dir: &std::path::Path, name: &str, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
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

    #[tokio::test]
    async fn file_backend_round_trips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path());
        assert_eq!(backend.load().await.unwrap(), None);
        let saved = state();
        backend.save(&saved).await.unwrap();

        let loaded = backend.load().await.unwrap().expect("saved state");
        assert_eq!(loaded.overrides, saved.overrides);
        assert_eq!(loaded.revisions.len(), 1);
        assert_eq!(loaded.revisions[0].source, crate::ChangeSource::Voice);
        assert!(backend.is_persistent());
    }

    #[tokio::test]
    async fn file_backend_creates_its_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path().join("nested/config"));
        backend.save(&state()).await.unwrap();
        assert!(backend.load().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_damaged_journal_costs_history_not_overrides() {
        let tmp = tempfile::TempDir::new().unwrap();
        let backend = FileBackend::new(tmp.path());
        backend.save(&state()).await.unwrap();
        std::fs::write(tmp.path().join(REVISIONS_FILE), "not = = toml").unwrap();

        let loaded = backend.load().await.unwrap().expect("overrides still load");
        assert_eq!(loaded.overrides, state().overrides);
        assert!(loaded.revisions.is_empty());
    }

    #[tokio::test]
    async fn an_unparseable_override_document_is_an_error_not_an_empty_state() {
        // Silently reporting "nothing stored" would look identical to a
        // fresh install and quietly discard the user's tuning.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(OVERRIDES_FILE), "not = = toml").unwrap();
        assert!(FileBackend::new(tmp.path()).load().await.is_err());
    }
}

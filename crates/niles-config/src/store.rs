//! Layered, hot-swappable configuration.
//!
//! [`Config`] on its own is a snapshot of one TOML file, read once. That
//! is the right shape for structural settings — brokers, listen
//! addresses, credentials — which are established at startup and cannot
//! meaningfully change while the process runs.
//!
//! It is the wrong shape for the values you tune by living with them:
//! the lighting curve above all. Those live here instead.
//!
//! # Layering
//!
//! The file Niles boots from is the **base** — in the cluster, a
//! read-only ConfigMap that Flux owns and reconciles. On top of it sits
//! an **overrides** document: a partial TOML tree holding only the values
//! that have been changed away from the base, deep-merged over it before
//! deserialization.
//!
//! ```text
//!   niles.toml (ConfigMap, read-only)   ← defaults, owned by git
//!         +
//!   overrides.toml (writable dir)       ← only what was changed
//!         =
//!   effective Config                    ← always validated
//! ```
//!
//! Nothing here ever writes to the base. The override document is the
//! only thing that changes, so Flux reconciling the ConfigMap and a user
//! changing a value cannot fight each other, and "reset to the file
//! value" is just removing a key from the overrides.
//!
//! # Validation
//!
//! A write that produces a config `validate()` rejects is refused and the
//! store keeps its previous state. The effective config is therefore
//! valid at every observable moment, including after a rejected write.

use crate::backend::{FileBackend, MemoryBackend, OverrideBackend, StoredState};
use crate::error::{Error, Result};
use crate::{Config, Reload, section_reload};
use std::path::Path;
use std::sync::{Arc, RwLock};

/// How much history to keep. Long enough to walk back a session's worth
/// of tuning, short enough that the stored journal stays readable.
const MAX_REVISIONS: usize = 50;

/// Base config plus an override document, kept as a validated snapshot.
///
/// Cheap to clone the snapshot out of ([`current`](Self::current)), so
/// hot paths take one per tick rather than holding a lock.
pub struct ConfigStore {
    /// Parsed base, kept so overrides can be re-layered from scratch on
    /// every write — cheaper to reason about than unwinding a merge.
    base: toml::Table,
    /// Where the override document is kept. See [`OverrideBackend`].
    backend: Box<dyn OverrideBackend>,
    /// Held for the length of a write, which spans an await while the
    /// backend persists. `inner`'s `RwLock` cannot do that job: a std
    /// lock must not be held across an await, and two writers
    /// interleaving there would each validate against state the other was
    /// about to replace.
    write_lock: tokio::sync::Mutex<()>,
    /// Current state. Guarded together so a reader can never see
    /// overrides that disagree with the snapshot they produced.
    inner: RwLock<Inner>,
    /// Bumped after every change that lands, so a task driving real
    /// hardware can act on a new value instead of waiting out its own
    /// polling interval. A watch, not a `Notify`: a change that arrives
    /// between two waits must still be seen.
    changed: tokio::sync::watch::Sender<u64>,
}

struct Inner {
    overrides: toml::Table,
    current: Arc<Config>,
    revisions: Vec<Revision>,
}

impl ConfigStore {
    /// Load `base_path`, layer whatever `backend` holds, and validate.
    ///
    /// A backend that fails, or that holds a document producing an
    /// invalid config, is **not** fatal: it is reported through the
    /// returned [`LoadOutcome`] and the store falls back to the base
    /// alone. Refusing to boot because a tuning value was bad — or
    /// because a database was mid-failover — would take the whole house
    /// down with it. An invalid *base* is still fatal; there is nothing
    /// safe to fall back to.
    pub async fn open(
        base_path: impl AsRef<Path>,
        backend: Box<dyn OverrideBackend>,
    ) -> Result<(Self, LoadOutcome)> {
        let base_path = base_path.as_ref();
        let raw = std::fs::read_to_string(base_path).map_err(|source| Error::Read {
            path: base_path.to_path_buf(),
            source,
        })?;
        let base: toml::Table = toml::from_str(&raw)?;
        let base_config = deserialize_validated(&base)?;

        let mut outcome = LoadOutcome::Clean;
        let mut state = StoredState::default();
        let mut current = Arc::new(base_config);

        match backend.load().await {
            Ok(Some(stored)) => match layer(&base, &stored.overrides) {
                Ok(config) => {
                    state = stored;
                    current = Arc::new(config);
                }
                Err(e) => outcome = LoadOutcome::OverridesRejected(e.to_string()),
            },
            Ok(None) => {}
            Err(e) => outcome = LoadOutcome::BackendUnavailable(e),
        }

        Ok((
            Self {
                base,
                backend,
                write_lock: tokio::sync::Mutex::new(()),
                inner: RwLock::new(Inner {
                    overrides: state.overrides,
                    current,
                    revisions: state.revisions,
                }),
                changed: tokio::sync::watch::channel(0).0,
            },
            outcome,
        ))
    }

    /// A store over a directory of TOML files.
    pub async fn open_with_dir(
        base_path: impl AsRef<Path>,
        dir: impl Into<std::path::PathBuf>,
    ) -> Result<(Self, LoadOutcome)> {
        Self::open(base_path, Box::new(FileBackend::new(dir))).await
    }

    /// A store whose writes apply but are never persisted.
    pub async fn open_in_memory(base_path: impl AsRef<Path>) -> Result<(Self, LoadOutcome)> {
        Self::open(base_path, Box::new(MemoryBackend::new())).await
    }

    /// Build a store from TOML text, backed by memory. Writes apply and
    /// are lost on drop.
    pub fn from_str_in_memory(base_toml: &str) -> Result<Self> {
        let base: toml::Table = toml::from_str(base_toml)?;
        let config = deserialize_validated(&base)?;
        Ok(Self {
            base,
            backend: Box::new(MemoryBackend::new()),
            write_lock: tokio::sync::Mutex::new(()),
            inner: RwLock::new(Inner {
                overrides: toml::Table::new(),
                current: Arc::new(config),
                revisions: Vec::new(),
            }),
            changed: tokio::sync::watch::channel(0).0,
        })
    }

    /// Replace the in-memory state with `state`, validating first.
    ///
    /// For the case where a backend was unreachable at startup and has
    /// since come back. Without it, a pod that restarted during a
    /// database failover would run on base defaults for the rest of its
    /// life, silently ignoring everything the user had tuned.
    ///
    /// Does not write back — the state came *from* the backend.
    pub fn adopt(&self, state: StoredState) -> Result<()> {
        let config = layer(&self.base, &state.overrides)?;
        let mut guard = self.write();
        guard.overrides = state.overrides;
        guard.current = Arc::new(config);
        guard.revisions = state.revisions;
        Ok(())
    }

    /// Re-read the backend and adopt what it holds.
    ///
    /// Returns whether anything was found. Used by the retry loop that
    /// runs when the backend was unavailable at startup.
    pub async fn reload(&self) -> Result<bool> {
        let loaded = self.backend.load().await.map_err(|reason| Error::Backend {
            backend: self.backend.describe(),
            reason,
        })?;
        match loaded {
            Some(state) => {
                self.adopt(state)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// The effective config right now.
    ///
    /// Take one of these per tick and read from it, rather than calling
    /// repeatedly — that way a write landing mid-tick can't make one tick
    /// observe two different configs.
    pub fn current(&self) -> Arc<Config> {
        Arc::clone(&self.read().current)
    }

    /// The override document as it stands — only the values that differ
    /// from the base.
    pub fn overrides(&self) -> toml::Table {
        self.read().overrides.clone()
    }

    /// Base + overrides as a plain table: what you would see if you could
    /// read the merged file.
    ///
    /// The typed [`Config`] deliberately has no `Serialize`, so anything
    /// that needs to *render* the config — the HTTP API, and the UI
    /// behind it — works from this instead. It also means new sections
    /// show up without anyone maintaining a list.
    pub fn effective_table(&self) -> toml::Table {
        layer_table(&self.base, &self.read().overrides)
    }

    /// Wakes on every change that lands, so a task driving real
    /// hardware can apply a new value at once rather than waiting out
    /// its own polling interval.
    ///
    /// The value is the revision that caused it, which matters only for
    /// logging — a receiver should read [`Self::current`] rather than
    /// trust it.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Whether writes outlive the process.
    pub fn is_persistent(&self) -> bool {
        self.backend.is_persistent()
    }

    /// Human-readable description of where state is kept, for logs.
    pub fn backend_description(&self) -> String {
        self.backend.describe()
    }

    /// Merge `patch` into the overrides, validate, persist, and swap the
    /// snapshot.
    ///
    /// The returned [`Applied`] carries the per-value diff, so a caller
    /// can report *what actually changed* rather than echoing back what
    /// it asked for — the difference between a confirmation and a no-op
    /// nobody noticed. On any error the store is untouched.
    pub async fn apply(&self, patch: &toml::Table, source: ChangeSource) -> Result<Applied> {
        if patch.is_empty() {
            return Ok(Applied::empty());
        }
        self.commit(source, |overrides| {
            let mut next = overrides.clone();
            merge(&mut next, patch);
            (next, flatten(patch).into_iter().map(|(p, _)| p).collect())
        })
        .await
    }

    /// Drop the override for a dotted path, returning that value to
    /// whatever the base file says.
    ///
    /// Removing a key that isn't overridden is not an error — the end
    /// state is what the caller asked for either way — but it is also not
    /// a revision, since nothing changed.
    pub async fn reset(&self, path: &str, source: ChangeSource) -> Result<Applied> {
        self.commit(source, |overrides| {
            let mut next = overrides.clone();
            remove_path(&mut next, path);
            (next, vec![path.to_string()])
        })
        .await
    }

    /// Undo the most recent change, restoring the override document as it
    /// stood before it.
    ///
    /// Returns `None` when there is nothing to undo. Undo is a pop, not a
    /// new revision: undoing twice walks two changes back rather than
    /// oscillating between two states.
    pub async fn undo(&self) -> Result<Option<Applied>> {
        let _write = self.write_lock.lock().await;
        let (overrides, revisions) = self.snapshot();
        let Some(last) = revisions.last().cloned() else {
            return Ok(None);
        };

        let restored = last.overrides_before.clone();
        let config = layer(&self.base, &restored)?;
        let before = layer_table(&self.base, &overrides);
        let after = layer_table(&self.base, &restored);

        let mut remaining = revisions;
        remaining.pop();
        self.persist(&restored, &remaining).await?;

        // The diff of an undo is the inverse of the revision it removes.
        let changes: Vec<Change> = last
            .changed_paths
            .iter()
            .map(|path| Change {
                path: path.clone(),
                from: lookup(&before, path).cloned(),
                to: lookup(&after, path)
                    .cloned()
                    .unwrap_or(toml::Value::String(String::new())),
            })
            .collect();

        let mut guard = self.write();
        guard.overrides = restored;
        guard.current = Arc::new(config);
        guard.revisions = remaining;
        Ok(Some(Applied {
            revision: last.id,
            sections: sections_of(&changes),
            changes,
        }))
    }

    /// Every recorded change, oldest first.
    pub fn history(&self) -> Vec<Revision> {
        self.read().revisions.clone()
    }

    /// The single place any write becomes visible.
    ///
    /// `next` produces the new override document and the paths it claims
    /// to touch; everything after — validating, diffing, journalling,
    /// persisting, swapping — is identical for every caller.
    ///
    /// Ordering is deliberate: **persist before swapping**. A write that
    /// reached memory but not durable storage would silently revert on
    /// the next restart, which is worse than refusing it outright.
    async fn commit(
        &self,
        source: ChangeSource,
        next: impl FnOnce(&toml::Table) -> (toml::Table, Vec<String>),
    ) -> Result<Applied> {
        // Serialises writers across the await below. Without it, two
        // concurrent writes would each validate against a document the
        // other was about to replace, and the loser's change would vanish
        // with no error anywhere.
        let _write = self.write_lock.lock().await;
        let (overrides, revisions) = self.snapshot();

        let (next, paths) = next(&overrides);
        // Validate against the *base*, not the running config: the
        // override document is the whole delta, so this is what a fresh
        // boot would produce.
        let config = layer(&self.base, &next)?;

        // The one write that cannot be undone from the page that made
        // it: with nobody on the allowlist, nobody can sign in to put
        // somebody back. Refused here rather than in `validate`,
        // because an empty list is a perfectly valid *state* — it is
        // how a fresh install starts — and only the transition out of
        // a non-empty one is the mistake.
        if self.current().auth.would_lock_out(&config.auth) {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: "removing the last person would leave nobody able to sign in;                          add somebody else first"
                    .into(),
            });
        }

        let before = layer_table(&self.base, &overrides);
        let after = layer_table(&self.base, &next);
        let changes: Vec<Change> = paths
            .into_iter()
            .map(|path| Change {
                from: lookup(&before, &path).cloned(),
                to: lookup(&after, &path)
                    .cloned()
                    .unwrap_or(toml::Value::String(String::new())),
                path,
            })
            .filter(|c| c.from.as_ref() != Some(&c.to))
            .collect();
        if changes.is_empty() {
            // Setting a value to what it already was is not a revision;
            // journalling it would fill the undo history with no-ops.
            return Ok(Applied::empty());
        }

        let mut revisions = revisions;
        let id = revisions.last().map_or(1, |r| r.id + 1);
        revisions.push(Revision {
            id,
            at: chrono::Utc::now(),
            source,
            summary: summarize(&changes),
            changed_paths: changes.iter().map(|c| c.path.clone()).collect(),
            overrides_before: overrides,
        });
        if revisions.len() > MAX_REVISIONS {
            revisions.remove(0);
        }

        self.persist(&next, &revisions).await?;

        let mut guard = self.write();
        guard.overrides = next;
        guard.current = Arc::new(config);
        guard.revisions = revisions;
        drop(guard);
        // After the swap, so a woken reader sees the new config.
        self.changed.send_replace(id);
        Ok(Applied {
            revision: id,
            sections: sections_of(&changes),
            changes,
        })
    }

    /// Hand the whole state to the backend.
    ///
    /// Always a full replacement rather than an incremental edit, which
    /// makes the write idempotent: a retry after a dropped connection — a
    /// Postgres failover, say — cannot apply anything twice.
    async fn persist(&self, overrides: &toml::Table, revisions: &[Revision]) -> Result<()> {
        let state = StoredState {
            overrides: overrides.clone(),
            revisions: revisions.to_vec(),
        };
        self.backend
            .save(&state)
            .await
            .map_err(|reason| Error::Backend {
                backend: self.backend.describe(),
                reason,
            })
    }

    fn snapshot(&self) -> (toml::Table, Vec<Revision>) {
        let guard = self.read();
        (guard.overrides.clone(), guard.revisions.clone())
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// Who made a change. Recorded per revision so the history answers
/// "did I do that, or did Niles?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChangeSource {
    /// A spoken request, via the LLM tool.
    Voice,
    /// The config UI or a direct HTTP call.
    Api,
}

/// One value that changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Dotted path, e.g. `lighting.daytime_brightness`.
    pub path: String,
    /// The effective value before — from the base or an earlier
    /// override. `None` if the key wasn't set at all.
    pub from: Option<toml::Value>,
    pub to: toml::Value,
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.from {
            Some(from) => write!(f, "{} {} → {}", self.path, terse(from), terse(&self.to)),
            None => write!(f, "{} set to {}", self.path, terse(&self.to)),
        }
    }
}

/// The outcome of a write: what changed, which sections it touched, and
/// the revision it became.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// Revision id, or 0 when nothing changed.
    pub revision: u64,
    pub changes: Vec<Change>,
    pub sections: Vec<SectionChange>,
}

impl Applied {
    fn empty() -> Self {
        Self {
            revision: 0,
            changes: Vec::new(),
            sections: Vec::new(),
        }
    }

    /// True when the write was accepted but changed nothing — every value
    /// already held the requested setting.
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    /// Sections that changed but won't take effect until a restart.
    pub fn needs_restart(&self) -> Vec<&str> {
        self.sections
            .iter()
            .filter(|s| s.reload == Reload::Boot)
            .map(|s| s.section.as_str())
            .collect()
    }

    /// One-line description of the whole write, for speaking back or
    /// logging: `lighting.daytime_brightness 100 → 85`.
    pub fn summary(&self) -> String {
        summarize(&self.changes)
    }
}

/// A recorded change, kept so it can be undone and shown in a history.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Revision {
    pub id: u64,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: ChangeSource,
    pub summary: String,
    /// Paths this revision touched, used to describe an undo.
    pub changed_paths: Vec<String>,
    /// The override document as it stood *before* this change. Undo
    /// restores it wholesale — simpler to reason about than inverting a
    /// merge, and the documents are small.
    pub overrides_before: toml::Table,
}

/// What happened to the override document at startup.
///
/// Boot never fails on a bad override — the base still loads — but the
/// caller should say so loudly, because the user's tuning silently
/// reverted to file defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadOutcome {
    /// No overrides, or overrides applied cleanly.
    Clean,
    /// Overrides parsed but produced an invalid config; base used alone.
    OverridesRejected(String),
    /// The backend could not be reached, or its content could not be
    /// parsed; base used alone. Distinct from `OverridesRejected`: the
    /// stored document may be perfectly good and merely out of reach,
    /// which is transient and worth retrying.
    BackendUnavailable(String),
}

impl LoadOutcome {
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }
}

/// A top-level section touched by a write, and whether the running
/// process will actually pick the change up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionChange {
    pub section: String,
    pub reload: Reload,
}

impl SectionChange {
    fn new(section: &str) -> Self {
        Self {
            section: section.to_string(),
            reload: section_reload(section),
        }
    }
}

/// Flatten a patch into `(dotted path, value)` leaves, so a nested
/// `[lighting] daytime_brightness = 85` reads back as the single path
/// `lighting.daytime_brightness`.
///
/// Arrays are leaves: `color_temp_anchors` changes as a whole, and
/// reporting it per element would describe an edit nobody made.
fn flatten(table: &toml::Table) -> Vec<(String, toml::Value)> {
    fn walk(table: &toml::Table, prefix: &str, out: &mut Vec<(String, toml::Value)>) {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                toml::Value::Table(inner) => walk(inner, &path, out),
                _ => out.push((path, value.clone())),
            }
        }
    }
    let mut out = Vec::new();
    walk(table, "", &mut out);
    out
}

/// Resolve a dotted path within a table.
fn lookup<'t>(table: &'t toml::Table, path: &str) -> Option<&'t toml::Value> {
    let mut current = table.get(path.split('.').next()?)?;
    for segment in path.split('.').skip(1) {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

/// Render a TOML scalar without the quoting noise — this ends up spoken
/// aloud and printed in logs.
fn terse(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn summarize(changes: &[Change]) -> String {
    changes
        .iter()
        .map(Change::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Base + overrides as a plain table, for diffing. Skips deserialization
/// because a diff doesn't need a typed `Config`.
fn layer_table(base: &toml::Table, overrides: &toml::Table) -> toml::Table {
    let mut merged = base.clone();
    merge(&mut merged, overrides);
    merged
}

/// Deep-merge `patch` into `target`.
///
/// Tables recurse; every other value replaces wholesale. Arrays are
/// replaced rather than concatenated — for `color_temp_anchors`, element
/// -wise merging would produce a curve nobody asked for.
fn merge(target: &mut toml::Table, patch: &toml::Table) {
    for (key, value) in patch {
        match (target.get_mut(key), value) {
            (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
                merge(existing, incoming);
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Remove a dotted path (`lighting.daytime_brightness`) from a table.
/// Returns whether anything was removed.
fn remove_path(table: &mut toml::Table, path: &str) -> bool {
    let Some((head, rest)) = path.split_once('.') else {
        return table.remove(path).is_some();
    };
    let Some(toml::Value::Table(inner)) = table.get_mut(head) else {
        return false;
    };
    let removed = remove_path(inner, rest);
    // Don't leave an empty section behind: an empty table in the
    // override document reads as "something is overridden here" to both
    // the UI and a human opening the file.
    if removed && inner.is_empty() {
        table.remove(head);
    }
    removed
}

/// Base + overrides → a validated `Config`.
fn layer(base: &toml::Table, overrides: &toml::Table) -> Result<Config> {
    let mut merged = base.clone();
    merge(&mut merged, overrides);
    deserialize_validated(&merged)
}

fn deserialize_validated(table: &toml::Table) -> Result<Config> {
    let config: Config = table.clone().try_into()?;
    config.validate()?;
    Ok(config)
}

/// Top-level sections touched by a set of changes, in first-seen order,
/// each carrying whether the running process will pick it up.
fn sections_of(changes: &[Change]) -> Vec<SectionChange> {
    let mut sections: Vec<SectionChange> = Vec::new();
    for change in changes {
        let section = change.path.split('.').next().unwrap_or(&change.path);
        if !sections.iter().any(|s| s.section == section) {
            sections.push(SectionChange::new(section));
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{OVERRIDES_FILE, REVISIONS_FILE};
    use std::path::PathBuf;

    /// A minimal config that `validate()` accepts, used as the base in
    /// every test here.
    fn base_toml() -> &'static str {
        crate::tests_support::valid_toml()
    }

    fn patch(s: &str) -> toml::Table {
        toml::from_str(s).expect("test patch parses")
    }

    /// The minimal base plus an `[auth]` section with a GitHub app
    /// named, so `is_enabled` turns purely on who is listed.
    fn with_auth(allowed: &str) -> ConfigStore {
        let toml = format!(
            "{}
[auth]
github_client_id_env = \"NILES_GITHUB_CLIENT_ID\"
             github_client_secret_env = \"NILES_GITHUB_CLIENT_SECRET\"
{allowed}
",
            base_toml()
        );
        ConfigStore::from_str_in_memory(&toml).expect("fixture config is valid")
    }

    #[tokio::test]
    async fn without_overrides_the_base_is_the_effective_config() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.overrides().is_empty());
    }

    #[tokio::test]
    async fn the_last_person_cannot_be_removed() {
        // Nobody left on the allowlist means nobody can sign in to put
        // somebody back — and the page offering that button is the page
        // you would have to be signed in to reach.
        let store = with_auth(r#"allowed = [{ email = "a@example.com" }]"#);

        let error = store
            .apply(
                &patch(
                    "[auth]
allowed = []
",
                ),
                ChangeSource::Api,
            )
            .await
            .expect_err("should refuse");
        assert!(
            error.to_string().contains("nobody able to sign in"),
            "the refusal should say why: {error}"
        );
        assert_eq!(store.current().auth.allowed.len(), 1, "unchanged");
    }

    #[tokio::test]
    async fn removing_yourself_is_allowed_while_somebody_remains() {
        let store =
            with_auth(r#"allowed = [{ email = "a@example.com" }, { email = "b@example.com" }]"#);

        store
            .apply(
                &patch(
                    "[auth]
allowed = [{ email = \"b@example.com\" }]
",
                ),
                ChangeSource::Api,
            )
            .await
            .expect("one person left is fine");
        assert_eq!(store.current().auth.allowed.len(), 1);
    }

    #[tokio::test]
    async fn the_first_person_can_always_be_added() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[auth]
allowed = [{ email = \"a@example.com\" }]
",
                ),
                ChangeSource::Api,
            )
            .await
            .expect("adding the first person is how sign-in gets switched on");
        assert_eq!(store.current().auth.allowed.len(), 1);
    }

    #[tokio::test]
    async fn apply_changes_the_effective_config() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 85);
    }

    #[tokio::test]
    async fn apply_leaves_untouched_keys_at_their_base_values() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let before = store.current();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        let after = store.current();
        // Same section, different key: must survive the merge.
        assert_eq!(
            after.lighting.night_floor_brightness,
            before.lighting.night_floor_brightness
        );
        assert_eq!(after.lighting.morning_start, before.lighting.morning_start);
        // Unrelated section, untouched.
        assert_eq!(after.home.name, before.home.name);
    }

    #[tokio::test]
    async fn overrides_hold_only_the_changed_values() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        let overrides = store.overrides();
        let lighting = overrides["lighting"].as_table().unwrap();
        assert_eq!(lighting.len(), 1, "only the changed key is recorded");
        assert_eq!(lighting["daytime_brightness"].as_integer(), Some(85));
    }

    #[tokio::test]
    async fn successive_applies_accumulate() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        store
            .apply(
                &patch("[lighting]\nnight_floor_brightness = 5"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        let current = store.current();
        assert_eq!(current.lighting.daytime_brightness, 85);
        assert_eq!(current.lighting.night_floor_brightness, 5);
    }

    #[tokio::test]
    async fn an_invalid_write_is_refused_and_changes_nothing() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();

        // morning_start after morning_end — CurveConfig::validate rejects it.
        let err = store
            .apply(
                &patch("[lighting]\nmorning_start = \"09:00\""),
                ChangeSource::Api,
            )
            .await;
        assert!(err.is_err(), "invalid config must be refused");

        // The earlier, valid override is still in force.
        assert_eq!(store.current().lighting.daytime_brightness, 85);
        assert_eq!(
            store.overrides()["lighting"]
                .as_table()
                .unwrap()
                .get("morning_start"),
            None,
            "a refused write leaves no trace in the overrides"
        );
    }

    #[tokio::test]
    async fn reset_returns_a_value_to_the_base() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let original = store.current().lighting.daytime_brightness;
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .await
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, original);
    }

    #[tokio::test]
    async fn reset_prunes_the_emptied_section() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .await
            .unwrap();
        assert!(
            store.overrides().is_empty(),
            "an emptied section should not linger as an empty table"
        );
    }

    #[tokio::test]
    async fn reset_of_an_unoverridden_path_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .await
            .unwrap();
        assert!(store.overrides().is_empty());
    }

    #[tokio::test]
    async fn apply_reports_the_sections_it_touched() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        assert_eq!(changed.sections.len(), 1);
        assert_eq!(changed.sections[0].section, "lighting");
        assert_eq!(changed.sections[0].reload, Reload::Hot);
    }

    #[tokio::test]
    async fn apply_flags_a_section_the_running_process_wont_pick_up() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9090\""),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        assert_eq!(changed.sections[0].reload, Reload::Boot);
        assert_eq!(changed.needs_restart(), vec!["api"]);
    }

    #[tokio::test]
    async fn a_misspelled_key_is_refused_rather_than_silently_stored() {
        // `deny_unknown_fields` is what catches this. Without the
        // round-trip through `Config`, a typo would sit in the override
        // document forever, doing nothing and explaining nothing.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let err = store
            .apply(
                &patch("[api]\nbnid_address = \"0.0.0.0:9090\""),
                ChangeSource::Api,
            )
            .await
            .expect_err("unknown key must be refused");
        assert!(err.to_string().contains("bnid_address"));
        assert!(store.overrides().is_empty());
    }

    #[tokio::test]
    async fn an_empty_patch_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(
            store
                .apply(&toml::Table::new(), ChangeSource::Api)
                .await
                .unwrap()
                .is_noop()
        );
        assert!(store.overrides().is_empty());
    }

    #[tokio::test]
    async fn in_memory_store_reports_that_writes_are_not_persisted() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(!store.is_persistent());
    }

    // ---- diffs, no-ops and undo -------------------------------------------

    #[tokio::test]
    async fn apply_reports_the_value_it_replaced() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let applied = store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        assert_eq!(applied.changes.len(), 1);
        let change = &applied.changes[0];
        assert_eq!(change.path, "lighting.daytime_brightness");
        assert_eq!(change.from, Some(toml::Value::Integer(100)));
        assert_eq!(change.to, toml::Value::Integer(85));
        assert_eq!(applied.summary(), "lighting.daytime_brightness 100 → 85");
    }

    #[tokio::test]
    async fn a_nested_patch_reads_back_as_a_dotted_path() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let applied = store
            .apply(
                &patch(
                    "[lighting]
sunset_start = \"22:00\"",
                ),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        assert_eq!(applied.changes[0].path, "lighting.sunset_start");
        // Strings are spoken and logged, so they lose their quoting.
        assert_eq!(applied.summary(), "lighting.sunset_start 21:30 → 22:00");
    }

    #[tokio::test]
    async fn setting_a_value_to_what_it_already_is_changes_nothing() {
        // Otherwise "set brightness to 100" when it is already 100 would
        // report success and fill the undo history with no-ops.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let applied = store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 100",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        assert!(applied.is_noop());
        assert_eq!(applied.revision, 0);
        assert!(store.history().is_empty());
        assert!(store.overrides().is_empty());
    }

    #[tokio::test]
    async fn each_accepted_write_becomes_a_revision() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
night_floor_brightness = 5",
                ),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        let history = store.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, 1);
        assert_eq!(history[0].source, ChangeSource::Voice);
        assert_eq!(history[1].id, 2);
        assert_eq!(history[1].source, ChangeSource::Api);
        assert_eq!(history[1].summary, "lighting.night_floor_brightness 15 → 5");
    }

    #[tokio::test]
    async fn a_refused_write_is_not_journalled() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let _ = store
            .apply(
                &patch(
                    "[lighting]
morning_start = \"09:00\"",
                ),
                ChangeSource::Voice,
            )
            .await;
        assert!(store.history().is_empty());
    }

    #[tokio::test]
    async fn undo_restores_the_previous_value() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        let undone = store.undo().await.unwrap().expect("something to undo");
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert_eq!(undone.changes[0].from, Some(toml::Value::Integer(85)));
        assert_eq!(undone.changes[0].to, toml::Value::Integer(100));
        assert!(store.history().is_empty(), "undo pops the revision");
    }

    #[tokio::test]
    async fn undo_walks_back_one_change_at_a_time() {
        // Not an oscillation between two states: undoing twice should
        // land on the original, not back on the first change.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 70",
                ),
                ChangeSource::Voice,
            )
            .await
            .unwrap();
        store.undo().await.unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 85);
        store.undo().await.unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(
            store.undo().await.unwrap().is_none(),
            "nothing left to undo"
        );
    }

    #[tokio::test]
    async fn a_change_wakes_a_subscriber() {
        // Without this a tuning change sits until the curve loop's next
        // tick — up to a minute of watching a light not do what you
        // just told it.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let mut changes = store.subscribe();
        assert!(
            !changes.has_changed().unwrap(),
            "quiet until something happens"
        );

        store
            .apply(
                &toml::from_str(
                    "[lighting]
daytime_brightness = 85",
                )
                .unwrap(),
                ChangeSource::Api,
            )
            .await
            .unwrap();

        assert!(changes.has_changed().unwrap());
        changes.changed().await.unwrap();
        assert_eq!(
            store.current().lighting.daytime_brightness,
            85,
            "and the new config is in force by the time it wakes"
        );
    }

    #[tokio::test]
    async fn setting_a_value_to_what_it_already_is_wakes_nobody() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changes = store.subscribe();
        store
            .apply(
                &toml::from_str(
                    "[lighting]
daytime_brightness = 100",
                )
                .unwrap(),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        assert!(!changes.has_changed().unwrap());
    }

    #[tokio::test]
    async fn undo_of_nothing_is_none_rather_than_an_error() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(store.undo().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reset_is_journalled_and_undoable() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .await
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        store.undo().await.unwrap();
        assert_eq!(
            store.current().lighting.daytime_brightness,
            85,
            "undoing a reset puts the override back"
        );
    }

    #[tokio::test]
    async fn history_is_capped() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        for n in 0..(MAX_REVISIONS + 10) {
            let value = 20 + (n % 60);
            store
                .apply(
                    &patch(&format!(
                        "[lighting]
daytime_brightness = {value}"
                    )),
                    ChangeSource::Api,
                )
                .await
                .unwrap();
        }
        let history = store.history();
        assert_eq!(history.len(), MAX_REVISIONS);
        assert!(history[0].id > 1, "oldest revisions are dropped");
    }

    // ---- on-disk behaviour ------------------------------------------------

    /// A store backed by a temp dir, with the base written to a file the
    /// way the cluster mounts it.
    fn on_disk() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::TempDir::new().unwrap();
        let base_path = tmp.path().join("niles.toml");
        std::fs::write(&base_path, base_toml()).unwrap();
        let dir = tmp.path().join("config");
        (tmp, base_path, dir)
    }

    #[tokio::test]
    async fn open_without_an_override_file_uses_the_base() {
        let (_tmp, base_path, dir) = on_disk();
        let (store, outcome) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(outcome.is_clean());
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.is_persistent());
    }

    #[tokio::test]
    async fn writes_survive_a_reopen() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
                .await
                .unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .await
                .unwrap();
        }
        let (reopened, outcome) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(outcome.is_clean());
        assert_eq!(reopened.current().lighting.daytime_brightness, 72);
    }

    #[tokio::test]
    async fn a_reopened_store_still_tracks_the_base_for_untouched_keys() {
        // The override file holds only the delta, so a later change to
        // the ConfigMap must still flow through on the next boot.
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
                .await
                .unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .await
                .unwrap();
        }
        // Simulate Flux reconciling a new base with a different floor.
        let edited =
            base_toml().replace("night_floor_brightness = 15", "night_floor_brightness = 8");
        std::fs::write(&base_path, edited).unwrap();

        let (reopened, _) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        let current = reopened.current();
        assert_eq!(
            current.lighting.night_floor_brightness, 8,
            "base change lands"
        );
        assert_eq!(
            current.lighting.daytime_brightness, 72,
            "override still wins"
        );
    }

    #[tokio::test]
    async fn unparseable_overrides_do_not_stop_the_boot() {
        let (_tmp, base_path, dir) = on_disk();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(OVERRIDES_FILE), "this is not = = toml").unwrap();

        let (store, outcome) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(matches!(outcome, LoadOutcome::BackendUnavailable(_)));
        assert_eq!(
            store.current().lighting.daytime_brightness,
            100,
            "falls back to the base rather than refusing to start"
        );
    }

    #[tokio::test]
    async fn overrides_that_fail_validation_do_not_stop_the_boot() {
        let (_tmp, base_path, dir) = on_disk();
        std::fs::create_dir_all(&dir).unwrap();
        // Parses fine, but inverts the morning ramp.
        std::fs::write(
            dir.join(OVERRIDES_FILE),
            "[lighting]\nmorning_start = \"09:00\"\n",
        )
        .unwrap();

        let (store, outcome) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(matches!(outcome, LoadOutcome::OverridesRejected(_)));
        assert_eq!(store.current().lighting.morning_start, "05:45");
    }

    #[tokio::test]
    async fn an_invalid_base_is_fatal() {
        // The override document is tuning; the base is the contract. A
        // broken base has no safe fallback to degrade to.
        //
        // Broken means *wrong*, not merely incomplete: every section has
        // a default now, so a base saying only the home's name is a
        // perfectly good base. This one names an address nothing can
        // bind to.
        let tmp = tempfile::TempDir::new().unwrap();
        let base_path = tmp.path().join("niles.toml");
        std::fs::write(&base_path, "[api]\nbind_address = \"not-an-address\"\n").unwrap();
        assert!(ConfigStore::open_in_memory(&base_path).await.is_err());
    }

    #[tokio::test]
    async fn reset_is_persisted_too() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
                .await
                .unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .await
                .unwrap();
            store
                .reset("lighting.daytime_brightness", ChangeSource::Api)
                .await
                .unwrap();
        }
        let (reopened, _) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert_eq!(reopened.current().lighting.daytime_brightness, 100);
    }

    #[tokio::test]
    async fn a_refused_write_leaves_the_file_alone() {
        let (_tmp, base_path, dir) = on_disk();
        let (store, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
            .await
            .unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 72"),
                ChangeSource::Api,
            )
            .await
            .unwrap();
        let _ = store
            .apply(
                &patch("[lighting]\nmorning_start = \"09:00\""),
                ChangeSource::Api,
            )
            .await;

        let raw = std::fs::read_to_string(dir.join(OVERRIDES_FILE)).unwrap();
        assert!(raw.contains("72"));
        assert!(
            !raw.contains("09:00"),
            "a rejected value must not reach disk"
        );
    }

    #[tokio::test]
    async fn history_survives_a_reopen_and_can_still_be_undone() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
                .await
                .unwrap();
            store
                .apply(
                    &patch(
                        "[lighting]
daytime_brightness = 72",
                    ),
                    ChangeSource::Voice,
                )
                .await
                .unwrap();
        }
        let (reopened, _) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        let history = reopened.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, ChangeSource::Voice);
        // The undo must work across the restart, not just in the session
        // that made the change.
        reopened
            .undo()
            .await
            .unwrap()
            .expect("undoable after reopen");
        assert_eq!(reopened.current().lighting.daytime_brightness, 100);
    }

    #[tokio::test]
    async fn a_damaged_journal_costs_history_not_config() {
        let (_tmp, base_path, dir) = on_disk();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(OVERRIDES_FILE),
            "[lighting]
daytime_brightness = 72
",
        )
        .unwrap();
        std::fs::write(dir.join(REVISIONS_FILE), "not = = toml").unwrap();

        let (store, outcome) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(outcome.is_clean(), "the override document is still fine");
        assert_eq!(store.current().lighting.daytime_brightness, 72);
        assert!(store.history().is_empty());
    }

    // ---- an unreachable backend -------------------------------------------

    /// A backend that is simply not there.
    struct BrokenBackend;

    #[async_trait::async_trait]
    impl OverrideBackend for BrokenBackend {
        async fn load(&self) -> std::result::Result<Option<StoredState>, String> {
            Err("connection refused".into())
        }
        async fn save(&self, _: &StoredState) -> std::result::Result<(), String> {
            Err("connection refused".into())
        }
        fn describe(&self) -> String {
            "the broken backend".into()
        }
    }

    #[tokio::test]
    async fn an_unreachable_backend_does_not_stop_the_boot() {
        let (_tmp, base_path, _dir) = on_disk();
        let (store, outcome) = ConfigStore::open(&base_path, Box::new(BrokenBackend))
            .await
            .unwrap();
        assert!(matches!(outcome, LoadOutcome::BackendUnavailable(_)));
        assert_eq!(
            store.current().lighting.daytime_brightness,
            100,
            "runs on base config rather than refusing to start"
        );
    }

    #[tokio::test]
    async fn a_write_to_an_unreachable_backend_fails_and_changes_nothing() {
        // The alternative — applying in memory and hoping — would show the
        // user a change that vanishes at the next restart.
        let (_tmp, base_path, _dir) = on_disk();
        let (store, _) = ConfigStore::open(&base_path, Box::new(BrokenBackend))
            .await
            .unwrap();
        let err = store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Api,
            )
            .await
            .expect_err("write must fail when it cannot be persisted");
        assert!(err.to_string().contains("connection refused"));
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.history().is_empty());
    }

    #[tokio::test]
    async fn adopt_picks_up_state_that_arrives_late() {
        // The repair path for a pod that started while its backend was
        // down: without it the house runs on defaults until someone
        // notices and restarts it.
        let (_tmp, base_path, _dir) = on_disk();
        let (store, _) = ConfigStore::open(&base_path, Box::new(BrokenBackend))
            .await
            .unwrap();
        store
            .adopt(StoredState {
                overrides: patch(
                    "[lighting]
daytime_brightness = 60",
                ),
                revisions: Vec::new(),
            })
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 60);
    }

    #[tokio::test]
    async fn adopt_refuses_state_that_would_not_validate() {
        let (_tmp, base_path, _dir) = on_disk();
        let (store, _) = ConfigStore::open(&base_path, Box::new(BrokenBackend))
            .await
            .unwrap();
        assert!(
            store
                .adopt(StoredState {
                    overrides: patch(
                        "[lighting]
morning_start = \"09:00\""
                    ),
                    revisions: Vec::new(),
                })
                .is_err()
        );
        assert_eq!(store.current().lighting.morning_start, "05:45");
    }

    #[tokio::test]
    async fn reload_adopts_what_the_backend_holds() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (writer, _) = ConfigStore::open_with_dir(&base_path, dir.clone())
                .await
                .unwrap();
            writer
                .apply(
                    &patch(
                        "[lighting]
daytime_brightness = 42",
                    ),
                    ChangeSource::Api,
                )
                .await
                .unwrap();
        }
        let (store, _) = ConfigStore::open_with_dir(&base_path, dir).await.unwrap();
        assert!(store.reload().await.unwrap());
        assert_eq!(store.current().lighting.daytime_brightness, 42);
    }

    // ---- merge unit tests -------------------------------------------------

    #[test]
    fn merge_recurses_into_tables() {
        let mut target = patch("[a]\nx = 1\ny = 2");
        merge(&mut target, &patch("[a]\ny = 3"));
        let a = target["a"].as_table().unwrap();
        assert_eq!(a["x"].as_integer(), Some(1));
        assert_eq!(a["y"].as_integer(), Some(3));
    }

    #[test]
    fn merge_replaces_arrays_wholesale() {
        // Element-wise merging of color_temp_anchors would interleave two
        // curves into one nobody configured.
        let mut target = patch("xs = [1, 2, 3]");
        merge(&mut target, &patch("xs = [9]"));
        assert_eq!(target["xs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_replaces_a_table_with_a_scalar() {
        let mut target = patch("[a]\nx = 1");
        merge(&mut target, &patch("a = 5"));
        assert_eq!(target["a"].as_integer(), Some(5));
    }

    #[test]
    fn remove_path_handles_nesting_and_absence() {
        let mut table = patch("[a.b]\nx = 1\ny = 2");
        assert!(remove_path(&mut table, "a.b.x"));
        assert!(!remove_path(&mut table, "a.b.zzz"));
        assert!(!remove_path(&mut table, "nope.nope"));
        // `y` remains, so the section stays.
        assert!(table.contains_key("a"));
    }
}

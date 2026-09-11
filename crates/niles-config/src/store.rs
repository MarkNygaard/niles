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

use crate::error::{Error, Result};
use crate::{Config, Reload, section_reload};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The file name of the override document inside the store's directory.
const OVERRIDES_FILE: &str = "overrides.toml";

/// Companion journal, alongside the override document.
const REVISIONS_FILE: &str = "revisions.toml";

/// How much history to keep. Long enough to walk back a session's worth
/// of tuning, short enough that the file stays readable by a human.
const MAX_REVISIONS: usize = 50;

/// Base config plus an override document, kept as a validated snapshot.
///
/// Cheap to clone the snapshot out of ([`current`](Self::current)), so
/// hot paths take one per tick rather than holding a lock.
pub struct ConfigStore {
    /// Parsed base, kept so overrides can be re-layered from scratch on
    /// every write — cheaper to reason about than unwinding a merge.
    base: toml::Table,
    /// Where `overrides.toml` lives. `None` disables persistence: writes
    /// apply in memory and are lost on restart. That is the right
    /// behaviour for tests and for a deployment with no writable volume,
    /// and it is reported by [`is_persistent`](Self::is_persistent) so
    /// callers can say so out loud rather than silently losing edits.
    dir: Option<PathBuf>,
    /// Current override document. Guarded together with `current` so a
    /// reader can never see overrides that disagree with the snapshot.
    inner: RwLock<Inner>,
}

struct Inner {
    overrides: toml::Table,
    current: Arc<Config>,
    revisions: Vec<Revision>,
}

impl ConfigStore {
    /// Load `base_path`, layer any overrides found in `dir`, and validate
    /// the result.
    ///
    /// A malformed or rejected override document is **not** fatal: it is
    /// reported through the returned [`LoadOutcome`] and the store falls
    /// back to the base alone. Refusing to boot because a tuning value
    /// was bad would take the whole house down over a brightness number.
    pub fn open(base_path: impl AsRef<Path>, dir: Option<PathBuf>) -> Result<(Self, LoadOutcome)> {
        let base_path = base_path.as_ref();
        let raw = std::fs::read_to_string(base_path).map_err(|source| Error::Read {
            path: base_path.to_path_buf(),
            source,
        })?;
        let base: toml::Table = toml::from_str(&raw)?;

        // The base alone must be valid — that one *is* fatal.
        let base_config = deserialize_validated(&base)?;

        let mut outcome = LoadOutcome::Clean;
        let mut overrides = toml::Table::new();
        let mut current = Arc::new(base_config);

        if let Some(dir) = &dir {
            match read_overrides(dir) {
                Ok(Some(found)) => match layer(&base, &found) {
                    Ok(config) => {
                        overrides = found;
                        current = Arc::new(config);
                    }
                    Err(e) => outcome = LoadOutcome::OverridesRejected(e.to_string()),
                },
                Ok(None) => {}
                Err(e) => outcome = LoadOutcome::OverridesUnreadable(e.to_string()),
            }
        }

        // A damaged journal costs history, not config — the override
        // document is the source of truth for what's in force.
        let revisions = dir
            .as_ref()
            .map(|dir| read_revisions(dir))
            .transpose()
            .unwrap_or_else(|e| {
                tracing_unavailable(&format!("could not read config revisions: {e}"));
                None
            })
            .flatten()
            .unwrap_or_default();

        Ok((
            Self {
                base,
                dir,
                inner: RwLock::new(Inner {
                    overrides,
                    current,
                    revisions,
                }),
            },
            outcome,
        ))
    }

    /// Build a store from TOML text with no backing directory. Writes
    /// apply in memory only.
    pub fn from_str_in_memory(base_toml: &str) -> Result<Self> {
        let base: toml::Table = toml::from_str(base_toml)?;
        let config = deserialize_validated(&base)?;
        Ok(Self {
            base,
            dir: None,
            inner: RwLock::new(Inner {
                overrides: toml::Table::new(),
                current: Arc::new(config),
                revisions: Vec::new(),
            }),
        })
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

    /// Whether writes survive a restart. False when the store has no
    /// directory, e.g. a deployment with no writable volume.
    pub fn is_persistent(&self) -> bool {
        self.dir.is_some()
    }

    /// Merge `patch` into the overrides, validate, and swap the snapshot.
    ///
    /// The returned [`Applied`] carries the per-value diff, so a caller
    /// can report *what actually changed* rather than echoing back what
    /// it asked for — the difference between a confirmation and a
    /// no-op nobody noticed. On any error the store is untouched.
    pub fn apply(&self, patch: &toml::Table, source: ChangeSource) -> Result<Applied> {
        if patch.is_empty() {
            return Ok(Applied::empty());
        }
        let mut guard = self.write();
        let mut next = guard.overrides.clone();
        merge(&mut next, patch);
        self.commit(&mut guard, next, source, |before, after| {
            // Diff against the *effective* config, not the override
            // document: "from" should be the value the user actually had,
            // whether it came from the base or an earlier override.
            flatten(patch)
                .into_iter()
                .map(|(path, to)| Change {
                    from: lookup(before, &path).cloned(),
                    to: lookup(after, &path).cloned().unwrap_or(to),
                    path,
                })
                .collect()
        })
    }

    /// Drop the override for a dotted path, returning that value to
    /// whatever the base file says.
    ///
    /// Removing a key that isn't overridden is not an error — the end
    /// state is what the caller asked for either way — but it is also not
    /// a revision, since nothing changed.
    pub fn reset(&self, path: &str, source: ChangeSource) -> Result<Applied> {
        let mut guard = self.write();
        let mut next = guard.overrides.clone();
        if !remove_path(&mut next, path) {
            return Ok(Applied::empty());
        }
        self.commit(&mut guard, next, source, |before, after| {
            vec![Change {
                from: lookup(before, path).cloned(),
                to: lookup(after, path)
                    .cloned()
                    .unwrap_or(toml::Value::String(String::new())),
                path: path.to_string(),
            }]
        })
    }

    /// Undo the most recent change, restoring the override document as it
    /// stood before it.
    ///
    /// Returns `None` when there is nothing to undo. Undo is a pop, not a
    /// new revision: undoing twice walks two changes back rather than
    /// oscillating between two states.
    pub fn undo(&self) -> Result<Option<Applied>> {
        let mut guard = self.write();
        let Some(last) = guard.revisions.last().cloned() else {
            return Ok(None);
        };
        let restored = last.overrides_before.clone();
        let before = layer_table(&self.base, &guard.overrides);
        let after = layer_table(&self.base, &restored);
        let config = layer(&self.base, &restored)?;

        let mut revisions = guard.revisions.clone();
        revisions.pop();
        if let Some(dir) = &self.dir {
            write_overrides(dir, &restored)?;
            write_revisions(dir, &revisions)?;
        }

        // The diff of an undo is the inverse of the revision it removes.
        let changes = last
            .changed_paths
            .iter()
            .filter_map(|path| {
                Some(Change {
                    path: path.clone(),
                    from: lookup(&before, path).cloned(),
                    to: lookup(&after, path).cloned()?,
                })
            })
            .collect();

        guard.overrides = restored;
        guard.current = Arc::new(config);
        guard.revisions = revisions;
        Ok(Some(Applied {
            revision: last.id,
            sections: last
                .changed_paths
                .iter()
                .filter_map(|p| p.split('.').next())
                .map(SectionChange::new)
                .collect(),
            changes,
        }))
    }

    /// Every recorded change, oldest first.
    pub fn history(&self) -> Vec<Revision> {
        self.read().revisions.clone()
    }

    /// Validate `next`, persist it, swap the snapshot, and journal the
    /// change. The single place any write becomes visible.
    ///
    /// `diff` is handed the effective config table before and after, so
    /// each caller describes its own change without duplicating the
    /// commit sequence.
    fn commit(
        &self,
        guard: &mut std::sync::RwLockWriteGuard<'_, Inner>,
        next: toml::Table,
        source: ChangeSource,
        diff: impl FnOnce(&toml::Table, &toml::Table) -> Vec<Change>,
    ) -> Result<Applied> {
        // Validate against the *base*, not the running config: the
        // override document is the whole delta, so this is what a fresh
        // boot would produce.
        let config = layer(&self.base, &next)?;

        let before = layer_table(&self.base, &guard.overrides);
        let after = layer_table(&self.base, &next);
        let changes: Vec<Change> = diff(&before, &after)
            .into_iter()
            .filter(|c| c.from.as_ref() != Some(&c.to))
            .collect();
        if changes.is_empty() {
            // Setting a value to what it already was is not a revision;
            // journalling it would fill the undo history with no-ops.
            return Ok(Applied::empty());
        }

        let mut revisions = guard.revisions.clone();
        let id = revisions.last().map_or(1, |r| r.id + 1);
        revisions.push(Revision {
            id,
            at: chrono::Utc::now(),
            source,
            summary: summarize(&changes),
            changed_paths: changes.iter().map(|c| c.path.clone()).collect(),
            overrides_before: guard.overrides.clone(),
        });
        if revisions.len() > MAX_REVISIONS {
            revisions.remove(0);
        }

        // Persist before swapping. A write that reaches memory but not
        // disk would silently revert on the next restart, which is worse
        // than refusing it.
        if let Some(dir) = &self.dir {
            write_overrides(dir, &next)?;
            write_revisions(dir, &revisions)?;
        }

        guard.overrides = next;
        guard.current = Arc::new(config);
        guard.revisions = revisions;

        let mut sections: Vec<SectionChange> = Vec::new();
        for change in &changes {
            let section = change.path.split('.').next().unwrap_or(&change.path);
            if !sections.iter().any(|s| s.section == section) {
                sections.push(SectionChange::new(section));
            }
        }
        Ok(Applied {
            revision: id,
            changes,
            sections,
        })
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
    /// Overrides could not be read or parsed; base used alone.
    OverridesUnreadable(String),
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

fn read_overrides(dir: &Path) -> Result<Option<toml::Table>> {
    let path = dir.join(OVERRIDES_FILE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(toml::from_str(&raw)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read { path, source }),
    }
}

/// Write the override document via a temp file + rename, so a crash
/// mid-write can't leave a half-parsed file that fails the next boot.
fn write_overrides(dir: &Path, overrides: &toml::Table) -> Result<()> {
    let body = toml::to_string_pretty(overrides).map_err(|e| Error::InvalidSection {
        section: "overrides",
        reason: format!("could not serialize the override document: {e}"),
    })?;
    write_atomically(dir, OVERRIDES_FILE, &body)
}

fn write_atomically(dir: &Path, name: &str, body: &str) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|source| Error::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|source| Error::Read {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        Error::Read { path, source }
    })
}

/// The journal is stored as an array of tables so it stays readable and
/// diffable by hand, like everything else in the config directory.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RevisionFile {
    #[serde(default, rename = "revision")]
    revisions: Vec<Revision>,
}

fn read_revisions(dir: &Path) -> Result<Option<Vec<Revision>>> {
    let path = dir.join(REVISIONS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let file: RevisionFile = toml::from_str(&raw)?;
            Ok(Some(file.revisions))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read { path, source }),
    }
}

fn write_revisions(dir: &Path, revisions: &[Revision]) -> Result<()> {
    let file = RevisionFile {
        revisions: revisions.to_vec(),
    };
    let body = toml::to_string_pretty(&file).map_err(|e| Error::InvalidSection {
        section: "revisions",
        reason: format!("could not serialize the revision journal: {e}"),
    })?;
    write_atomically(dir, REVISIONS_FILE, &body)
}

/// `niles-config` has no logging dependency, and a damaged journal is not
/// worth adding one for — the caller still gets a working store.
fn tracing_unavailable(message: &str) {
    eprintln!("warning: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal config that `validate()` accepts, used as the base in
    /// every test here.
    fn base_toml() -> &'static str {
        crate::tests_support::valid_toml()
    }

    fn patch(s: &str) -> toml::Table {
        toml::from_str(s).expect("test patch parses")
    }

    #[test]
    fn without_overrides_the_base_is_the_effective_config() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn apply_changes_the_effective_config() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 85);
    }

    #[test]
    fn apply_leaves_untouched_keys_at_their_base_values() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let before = store.current();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
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

    #[test]
    fn overrides_hold_only_the_changed_values() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        let overrides = store.overrides();
        let lighting = overrides["lighting"].as_table().unwrap();
        assert_eq!(lighting.len(), 1, "only the changed key is recorded");
        assert_eq!(lighting["daytime_brightness"].as_integer(), Some(85));
    }

    #[test]
    fn successive_applies_accumulate() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        store
            .apply(
                &patch("[lighting]\nnight_floor_brightness = 5"),
                ChangeSource::Api,
            )
            .unwrap();
        let current = store.current();
        assert_eq!(current.lighting.daytime_brightness, 85);
        assert_eq!(current.lighting.night_floor_brightness, 5);
    }

    #[test]
    fn an_invalid_write_is_refused_and_changes_nothing() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();

        // morning_start after morning_end — CurveConfig::validate rejects it.
        let err = store.apply(
            &patch("[lighting]\nmorning_start = \"09:00\""),
            ChangeSource::Api,
        );
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

    #[test]
    fn reset_returns_a_value_to_the_base() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let original = store.current().lighting.daytime_brightness;
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, original);
    }

    #[test]
    fn reset_prunes_the_emptied_section() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .unwrap();
        assert!(
            store.overrides().is_empty(),
            "an emptied section should not linger as an empty table"
        );
    }

    #[test]
    fn reset_of_an_unoverridden_path_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .unwrap();
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn apply_reports_the_sections_it_touched() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                ChangeSource::Api,
            )
            .unwrap();
        assert_eq!(changed.sections.len(), 1);
        assert_eq!(changed.sections[0].section, "lighting");
        assert_eq!(changed.sections[0].reload, Reload::Hot);
    }

    #[test]
    fn apply_flags_a_section_the_running_process_wont_pick_up() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9090\""),
                ChangeSource::Api,
            )
            .unwrap();
        assert_eq!(changed.sections[0].reload, Reload::Boot);
        assert_eq!(changed.needs_restart(), vec!["api"]);
    }

    #[test]
    fn a_misspelled_key_is_refused_rather_than_silently_stored() {
        // `deny_unknown_fields` is what catches this. Without the
        // round-trip through `Config`, a typo would sit in the override
        // document forever, doing nothing and explaining nothing.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let err = store
            .apply(
                &patch("[api]\nbnid_address = \"0.0.0.0:9090\""),
                ChangeSource::Api,
            )
            .expect_err("unknown key must be refused");
        assert!(err.to_string().contains("bnid_address"));
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn an_empty_patch_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(
            store
                .apply(&toml::Table::new(), ChangeSource::Api)
                .unwrap()
                .is_noop()
        );
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn in_memory_store_reports_that_writes_are_not_persisted() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(!store.is_persistent());
    }

    // ---- diffs, no-ops and undo -------------------------------------------

    #[test]
    fn apply_reports_the_value_it_replaced() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let applied = store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .unwrap();
        assert_eq!(applied.changes.len(), 1);
        let change = &applied.changes[0];
        assert_eq!(change.path, "lighting.daytime_brightness");
        assert_eq!(change.from, Some(toml::Value::Integer(100)));
        assert_eq!(change.to, toml::Value::Integer(85));
        assert_eq!(applied.summary(), "lighting.daytime_brightness 100 → 85");
    }

    #[test]
    fn a_nested_patch_reads_back_as_a_dotted_path() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let applied = store
            .apply(
                &patch(
                    "[lighting]
sunset_start = \"22:00\"",
                ),
                ChangeSource::Api,
            )
            .unwrap();
        assert_eq!(applied.changes[0].path, "lighting.sunset_start");
        // Strings are spoken and logged, so they lose their quoting.
        assert_eq!(applied.summary(), "lighting.sunset_start 21:30 → 22:00");
    }

    #[test]
    fn setting_a_value_to_what_it_already_is_changes_nothing() {
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
            .unwrap();
        assert!(applied.is_noop());
        assert_eq!(applied.revision, 0);
        assert!(store.history().is_empty());
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn each_accepted_write_becomes_a_revision() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
night_floor_brightness = 5",
                ),
                ChangeSource::Api,
            )
            .unwrap();
        let history = store.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, 1);
        assert_eq!(history[0].source, ChangeSource::Voice);
        assert_eq!(history[1].id, 2);
        assert_eq!(history[1].source, ChangeSource::Api);
        assert_eq!(history[1].summary, "lighting.night_floor_brightness 15 → 5");
    }

    #[test]
    fn a_refused_write_is_not_journalled() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let _ = store.apply(
            &patch(
                "[lighting]
morning_start = \"09:00\"",
            ),
            ChangeSource::Voice,
        );
        assert!(store.history().is_empty());
    }

    #[test]
    fn undo_restores_the_previous_value() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Voice,
            )
            .unwrap();
        let undone = store.undo().unwrap().expect("something to undo");
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert_eq!(undone.changes[0].from, Some(toml::Value::Integer(85)));
        assert_eq!(undone.changes[0].to, toml::Value::Integer(100));
        assert!(store.history().is_empty(), "undo pops the revision");
    }

    #[test]
    fn undo_walks_back_one_change_at_a_time() {
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
            .unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 70",
                ),
                ChangeSource::Voice,
            )
            .unwrap();
        store.undo().unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 85);
        store.undo().unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.undo().unwrap().is_none(), "nothing left to undo");
    }

    #[test]
    fn undo_of_nothing_is_none_rather_than_an_error() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(store.undo().unwrap().is_none());
    }

    #[test]
    fn reset_is_journalled_and_undoable() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(
                &patch(
                    "[lighting]
daytime_brightness = 85",
                ),
                ChangeSource::Api,
            )
            .unwrap();
        store
            .reset("lighting.daytime_brightness", ChangeSource::Api)
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        store.undo().unwrap();
        assert_eq!(
            store.current().lighting.daytime_brightness,
            85,
            "undoing a reset puts the override back"
        );
    }

    #[test]
    fn history_is_capped() {
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

    #[test]
    fn open_without_an_override_file_uses_the_base() {
        let (_tmp, base_path, dir) = on_disk();
        let (store, outcome) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert!(outcome.is_clean());
        assert_eq!(store.current().lighting.daytime_brightness, 100);
        assert!(store.is_persistent());
    }

    #[test]
    fn writes_survive_a_reopen() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .unwrap();
        }
        let (reopened, outcome) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert!(outcome.is_clean());
        assert_eq!(reopened.current().lighting.daytime_brightness, 72);
    }

    #[test]
    fn a_reopened_store_still_tracks_the_base_for_untouched_keys() {
        // The override file holds only the delta, so a later change to
        // the ConfigMap must still flow through on the next boot.
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .unwrap();
        }
        // Simulate Flux reconciling a new base with a different floor.
        let edited =
            base_toml().replace("night_floor_brightness = 15", "night_floor_brightness = 8");
        std::fs::write(&base_path, edited).unwrap();

        let (reopened, _) = ConfigStore::open(&base_path, Some(dir)).unwrap();
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

    #[test]
    fn unparseable_overrides_do_not_stop_the_boot() {
        let (_tmp, base_path, dir) = on_disk();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(OVERRIDES_FILE), "this is not = = toml").unwrap();

        let (store, outcome) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert!(matches!(outcome, LoadOutcome::OverridesUnreadable(_)));
        assert_eq!(
            store.current().lighting.daytime_brightness,
            100,
            "falls back to the base rather than refusing to start"
        );
    }

    #[test]
    fn overrides_that_fail_validation_do_not_stop_the_boot() {
        let (_tmp, base_path, dir) = on_disk();
        std::fs::create_dir_all(&dir).unwrap();
        // Parses fine, but inverts the morning ramp.
        std::fs::write(
            dir.join(OVERRIDES_FILE),
            "[lighting]\nmorning_start = \"09:00\"\n",
        )
        .unwrap();

        let (store, outcome) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert!(matches!(outcome, LoadOutcome::OverridesRejected(_)));
        assert_eq!(store.current().lighting.morning_start, "05:45");
    }

    #[test]
    fn an_invalid_base_is_fatal() {
        // The override document is tuning; the base is the contract. A
        // broken base has no safe fallback to degrade to.
        let tmp = tempfile::TempDir::new().unwrap();
        let base_path = tmp.path().join("niles.toml");
        std::fs::write(&base_path, "[home]\nname = \"only this\"\n").unwrap();
        assert!(ConfigStore::open(&base_path, None).is_err());
    }

    #[test]
    fn reset_is_persisted_too() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
            store
                .apply(
                    &patch("[lighting]\ndaytime_brightness = 72"),
                    ChangeSource::Api,
                )
                .unwrap();
            store
                .reset("lighting.daytime_brightness", ChangeSource::Api)
                .unwrap();
        }
        let (reopened, _) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert_eq!(reopened.current().lighting.daytime_brightness, 100);
    }

    #[test]
    fn a_refused_write_leaves_the_file_alone() {
        let (_tmp, base_path, dir) = on_disk();
        let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 72"),
                ChangeSource::Api,
            )
            .unwrap();
        let _ = store.apply(
            &patch("[lighting]\nmorning_start = \"09:00\""),
            ChangeSource::Api,
        );

        let raw = std::fs::read_to_string(dir.join(OVERRIDES_FILE)).unwrap();
        assert!(raw.contains("72"));
        assert!(
            !raw.contains("09:00"),
            "a rejected value must not reach disk"
        );
    }

    #[test]
    fn history_survives_a_reopen_and_can_still_be_undone() {
        let (_tmp, base_path, dir) = on_disk();
        {
            let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
            store
                .apply(
                    &patch(
                        "[lighting]
daytime_brightness = 72",
                    ),
                    ChangeSource::Voice,
                )
                .unwrap();
        }
        let (reopened, _) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        let history = reopened.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, ChangeSource::Voice);
        // The undo must work across the restart, not just in the session
        // that made the change.
        reopened.undo().unwrap().expect("undoable after reopen");
        assert_eq!(reopened.current().lighting.daytime_brightness, 100);
    }

    #[test]
    fn a_damaged_journal_costs_history_not_config() {
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

        let (store, outcome) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert!(outcome.is_clean(), "the override document is still fine");
        assert_eq!(store.current().lighting.daytime_brightness, 72);
        assert!(store.history().is_empty());
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

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

        Ok((
            Self {
                base,
                dir,
                inner: RwLock::new(Inner { overrides, current }),
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

    /// Whether writes survive a restart. False when the store has no
    /// directory, e.g. a deployment with no writable volume.
    pub fn is_persistent(&self) -> bool {
        self.dir.is_some()
    }

    /// Merge `patch` into the overrides, validate, and swap the snapshot.
    ///
    /// Returns the sections the patch touched, in the order they appear
    /// in the patch, so a caller can report what changed and warn about
    /// any that need a restart. On any error the store is untouched.
    pub fn apply(&self, patch: &toml::Table) -> Result<Vec<SectionChange>> {
        if patch.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self.write();

        let mut next = guard.overrides.clone();
        merge(&mut next, patch);
        // Validate against the *base*, not the running config: the
        // override document is the whole delta, so this is what a fresh
        // boot would produce.
        let config = layer(&self.base, &next)?;

        // Persist before swapping. A write that reaches memory but not
        // disk would silently revert on the next restart, which is worse
        // than refusing it.
        if let Some(dir) = &self.dir {
            write_overrides(dir, &next)?;
        }

        guard.overrides = next;
        guard.current = Arc::new(config);
        Ok(patch.keys().map(|k| SectionChange::new(k)).collect())
    }

    /// Drop the override for a dotted path, returning that value to
    /// whatever the base file says.
    ///
    /// Removing a key that isn't overridden is not an error — the
    /// end state is what the caller asked for either way.
    pub fn reset(&self, path: &str) -> Result<()> {
        let mut guard = self.write();
        let mut next = guard.overrides.clone();
        if !remove_path(&mut next, path) {
            return Ok(());
        }
        let config = layer(&self.base, &next)?;
        if let Some(dir) = &self.dir {
            write_overrides(dir, &next)?;
        }
        guard.overrides = next;
        guard.current = Arc::new(config);
        Ok(())
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
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
    std::fs::create_dir_all(dir).map_err(|source| Error::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(OVERRIDES_FILE);
    let tmp = dir.join(format!("{OVERRIDES_FILE}.tmp.{}", std::process::id()));
    let body = toml::to_string_pretty(overrides).map_err(|e| Error::InvalidSection {
        section: "overrides",
        reason: format!("could not serialize override document: {e}"),
    })?;
    std::fs::write(&tmp, body).map_err(|source| Error::Read {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        Error::Read { path, source }
    })
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
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, 85);
    }

    #[test]
    fn apply_leaves_untouched_keys_at_their_base_values() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let before = store.current();
        store
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
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
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
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
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();
        store
            .apply(&patch("[lighting]\nnight_floor_brightness = 5"))
            .unwrap();
        let current = store.current();
        assert_eq!(current.lighting.daytime_brightness, 85);
        assert_eq!(current.lighting.night_floor_brightness, 5);
    }

    #[test]
    fn an_invalid_write_is_refused_and_changes_nothing() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();

        // morning_start after morning_end — CurveConfig::validate rejects it.
        let err = store.apply(&patch("[lighting]\nmorning_start = \"09:00\""));
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
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();
        store.reset("lighting.daytime_brightness").unwrap();
        assert_eq!(store.current().lighting.daytime_brightness, original);
    }

    #[test]
    fn reset_prunes_the_emptied_section() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();
        store.reset("lighting.daytime_brightness").unwrap();
        assert!(
            store.overrides().is_empty(),
            "an emptied section should not linger as an empty table"
        );
    }

    #[test]
    fn reset_of_an_unoverridden_path_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        store.reset("lighting.daytime_brightness").unwrap();
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn apply_reports_the_sections_it_touched() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(&patch("[lighting]\ndaytime_brightness = 85"))
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].section, "lighting");
        assert_eq!(changed[0].reload, Reload::Hot);
    }

    #[test]
    fn apply_flags_a_section_the_running_process_wont_pick_up() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let changed = store
            .apply(&patch("[api]\nbind_address = \"0.0.0.0:9090\""))
            .unwrap();
        assert_eq!(changed[0].reload, Reload::Boot);
    }

    #[test]
    fn a_misspelled_key_is_refused_rather_than_silently_stored() {
        // `deny_unknown_fields` is what catches this. Without the
        // round-trip through `Config`, a typo would sit in the override
        // document forever, doing nothing and explaining nothing.
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        let err = store
            .apply(&patch("[api]\nbnid_address = \"0.0.0.0:9090\""))
            .expect_err("unknown key must be refused");
        assert!(err.to_string().contains("bnid_address"));
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn an_empty_patch_is_a_no_op() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(store.apply(&toml::Table::new()).unwrap().is_empty());
        assert!(store.overrides().is_empty());
    }

    #[test]
    fn in_memory_store_reports_that_writes_are_not_persisted() {
        let store = ConfigStore::from_str_in_memory(base_toml()).unwrap();
        assert!(!store.is_persistent());
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
                .apply(&patch("[lighting]\ndaytime_brightness = 72"))
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
                .apply(&patch("[lighting]\ndaytime_brightness = 72"))
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
                .apply(&patch("[lighting]\ndaytime_brightness = 72"))
                .unwrap();
            store.reset("lighting.daytime_brightness").unwrap();
        }
        let (reopened, _) = ConfigStore::open(&base_path, Some(dir)).unwrap();
        assert_eq!(reopened.current().lighting.daytime_brightness, 100);
    }

    #[test]
    fn a_refused_write_leaves_the_file_alone() {
        let (_tmp, base_path, dir) = on_disk();
        let (store, _) = ConfigStore::open(&base_path, Some(dir.clone())).unwrap();
        store
            .apply(&patch("[lighting]\ndaytime_brightness = 72"))
            .unwrap();
        let _ = store.apply(&patch("[lighting]\nmorning_start = \"09:00\""));

        let raw = std::fs::read_to_string(dir.join(OVERRIDES_FILE)).unwrap();
        assert!(raw.contains("72"));
        assert!(
            !raw.contains("09:00"),
            "a rejected value must not reach disk"
        );
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

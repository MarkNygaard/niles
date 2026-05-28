//! Scene store — snapshots and replays device state by name.
//!
//! Optional file persistence (JSON) survives process restarts.

use niles_core::{DeviceId, DeviceRegistry, DeviceState, RoomName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::persistence::{atomic_write_json, read_json_or_empty};
use crate::timer::canonicalize_name;

/// A single device captured in a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneEntry {
    pub device_id: DeviceId,
    pub state: DeviceState,
}

// ------------------------------------------------------------------
// Persistence DTOs
// ------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
struct DeviceStateDto {
    pub on: Option<bool>,
    pub brightness: Option<u8>,
    pub color_temp_kelvin: Option<u16>,
    pub temperature_celsius: Option<f32>,
    pub humidity_percent: Option<f32>,
    pub battery_percent: Option<u8>,
}

impl From<&DeviceState> for DeviceStateDto {
    fn from(s: &DeviceState) -> Self {
        Self {
            on: s.on,
            brightness: s.brightness,
            color_temp_kelvin: s.color_temp_kelvin,
            temperature_celsius: s.temperature_celsius,
            humidity_percent: s.humidity_percent,
            battery_percent: s.battery_percent,
        }
    }
}

impl From<DeviceStateDto> for DeviceState {
    fn from(d: DeviceStateDto) -> Self {
        Self {
            on: d.on,
            brightness: d.brightness,
            color_temp_kelvin: d.color_temp_kelvin,
            temperature_celsius: d.temperature_celsius,
            humidity_percent: d.humidity_percent,
            battery_percent: d.battery_percent,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedSceneEntry {
    device_id: String,
    state: DeviceStateDto,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedScenes {
    scenes: BTreeMap<String, Vec<PersistedSceneEntry>>,
}

/// In-memory store for named lighting scenes.
///
/// Mirrors the `RwLock<HashMap>` shape of [`ManualModeTracker`].
pub struct SceneStore {
    scenes: RwLock<HashMap<String, Vec<SceneEntry>>>,
    persistence_path: Option<PathBuf>,
}

impl SceneStore {
    pub fn new() -> Self {
        Self {
            scenes: RwLock::new(HashMap::new()),
            persistence_path: None,
        }
    }

    pub fn with_persistence(mut self, path: PathBuf) -> Self {
        self.persistence_path = Some(path);
        self
    }

    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let persisted: PersistedScenes = read_json_or_empty(path, "scenes")?;
        let mut scenes = HashMap::new();
        for (name, entries) in persisted.scenes {
            let valid: Vec<SceneEntry> = entries
                .into_iter()
                .filter_map(|e| match DeviceId::parse(&e.device_id) {
                    Ok(id) => Some(SceneEntry {
                        device_id: id,
                        state: e.state.into(),
                    }),
                    Err(_) => {
                        tracing::warn!(
                            "persistence: dropping scene entry with malformed device_id '{}'",
                            e.device_id
                        );
                        None
                    }
                })
                .collect();
            if !valid.is_empty() {
                scenes.insert(name, valid);
            }
        }
        Ok(Self {
            scenes: RwLock::new(scenes),
            persistence_path: None,
        })
    }

    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let inner = self.scenes_read();
        self.save_locked(&inner, path)
    }

    fn save_locked(
        &self,
        inner: &HashMap<String, Vec<SceneEntry>>,
        path: &Path,
    ) -> std::io::Result<()> {
        let scenes: BTreeMap<String, Vec<PersistedSceneEntry>> = inner
            .iter()
            .map(|(k, v)| {
                let entries = v
                    .iter()
                    .map(|e| PersistedSceneEntry {
                        device_id: e.device_id.to_string(),
                        state: (&e.state).into(),
                    })
                    .collect();
                (k.clone(), entries)
            })
            .collect();
        atomic_write_json(path, &PersistedScenes { scenes })
    }

    fn maybe_save(&self, inner: &HashMap<String, Vec<SceneEntry>>) {
        if let Some(path) = self.persistence_path.as_deref()
            && let Err(e) = self.save_locked(inner, path)
        {
            tracing::warn!("persistence: scenes save failed: {e}");
        }
    }

    /// Snapshot every *light* in `room` (or the whole registry if `room`
    /// is `None`) and store it under `name`. Returns the number of
    /// devices captured. Overwrites any existing scene with the same
    /// canonical name.
    ///
    /// Non-light devices are dropped — per ARCHITECTURE.md:491,
    /// scenes capture *lights*, not sensors, and `format_set_command`
    /// would emit an empty `{}` payload for sensor entries on apply
    /// anyway.
    pub fn save(&self, name: &str, registry: &DeviceRegistry, room: Option<&RoomName>) -> usize {
        let key = canonicalize_name(name);
        let entries: Vec<SceneEntry> = match room {
            Some(r) => registry.list_room(r),
            None => registry.list_all(),
        }
        .into_iter()
        .filter(|d| d.is_light())
        .map(|d| SceneEntry {
            device_id: d.id,
            state: d.state,
        })
        .collect();
        let n = entries.len();
        let mut inner = self.scenes_write();
        inner.insert(key, entries);
        self.maybe_save(&inner);
        n
    }

    /// Retrieve a previously saved scene. Returns `None` if no scene with
    /// the given (canonicalized) name exists.
    pub fn get(&self, name: &str) -> Option<Vec<SceneEntry>> {
        self.scenes_read().get(&canonicalize_name(name)).cloned()
    }

    /// True if a scene with the given name has been saved.
    pub fn exists(&self, name: &str) -> bool {
        self.scenes_read().contains_key(&canonicalize_name(name))
    }

    /// Return all saved scene names in lexicographic order.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.scenes_read().keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Remove a scene by name. Returns `true` if a scene with that
    /// (canonicalized) name was present and removed, `false` otherwise.
    pub fn delete(&self, name: &str) -> bool {
        let mut inner = self.scenes_write();
        let removed = inner.remove(&canonicalize_name(name)).is_some();
        self.maybe_save(&inner);
        removed
    }

    // ---- lock helpers -----------------------------------------------------

    fn scenes_write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Vec<SceneEntry>>> {
        self.scenes.write().unwrap_or_else(|e| e.into_inner())
    }

    fn scenes_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Vec<SceneEntry>>> {
        self.scenes.read().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for SceneStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{Device, DeviceClass};

    fn dev_in(room: &str, name: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{room}/{name}")).unwrap()
    }

    fn state(on: bool, brightness: u8, kelvin: u16) -> DeviceState {
        DeviceState {
            on: Some(on),
            brightness: Some(brightness),
            color_temp_kelvin: Some(kelvin),
            ..Default::default()
        }
    }

    fn registry_with(devs: &[(&str, &str, DeviceState)]) -> DeviceRegistry {
        let r = DeviceRegistry::new();
        for (room, name, s) in devs {
            let id = dev_in(room, name);
            r.upsert(Device {
                id,
                state: s.clone(),
                class: DeviceClass::Light,
            });
        }
        r
    }

    #[test]
    fn save_then_get_roundtrip() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        let n = store.save("evening", &reg, None);
        assert_eq!(n, 1);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
        assert_eq!(entries[0].state.on, Some(true));
    }

    #[test]
    fn save_with_some_room_filters() {
        let store = SceneStore::new();
        let reg = registry_with(&[
            ("kitchen", "a", state(true, 80, 2700)),
            ("bedroom", "b", state(false, 40, 3000)),
        ]);
        let kitchen = RoomName::parse("kitchen").unwrap();
        let n = store.save("evening", &reg, Some(&kitchen));
        assert_eq!(n, 1);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
    }

    #[test]
    fn save_with_none_captures_all() {
        let store = SceneStore::new();
        let reg = registry_with(&[
            ("kitchen", "a", state(true, 80, 2700)),
            ("bedroom", "b", state(false, 40, 3000)),
        ]);
        let n = store.save("all", &reg, None);
        assert_eq!(n, 2);
        assert_eq!(store.get("all").unwrap().len(), 2);
    }

    #[test]
    fn save_returns_entry_count() {
        let store = SceneStore::new();
        let reg = registry_with(&[
            ("kitchen", "a", state(true, 80, 2700)),
            ("kitchen", "b", state(true, 60, 2700)),
        ]);
        assert_eq!(store.save("dup", &reg, None), 2);
    }

    #[test]
    fn save_overwrites_by_name() {
        let store = SceneStore::new();
        let reg1 = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        let reg2 = registry_with(&[
            ("kitchen", "a", state(false, 20, 3000)),
            ("bedroom", "b", state(true, 100, 2700)),
        ]);
        store.save("evening", &reg1, None);
        let n = store.save("evening", &reg2, None);
        assert_eq!(n, 2);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn exists_lifecycle() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        assert!(!store.exists("evening"));
        store.save("evening", &reg, None);
        assert!(store.exists("evening"));
        assert!(!store.exists("morning"));
    }

    #[test]
    fn name_canonicalization_collisions() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("Kitchen Evening", &reg, None);
        assert!(store.exists("kitchen evening"));
        assert!(store.exists("  KITCHEN  EVENING  "));
    }

    #[test]
    fn names_returns_sorted() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("b", &reg, None);
        store.save("a", &reg, None);
        store.save("c", &reg, None);
        assert_eq!(store.names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn arc_clones_share_writes() {
        use std::sync::Arc;
        let store = Arc::new(SceneStore::new());
        let store2 = store.clone();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);
        assert!(store2.exists("evening"));
    }

    #[test]
    fn save_drops_non_light_devices() {
        // Sensor-style device: only sensor fields set. Scenes are
        // about lights (ARCHITECTURE.md:491), so it should be filtered
        // by `DeviceClass`, not just by state fields.
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        reg.upsert(Device {
            id: dev_in("kitchen", "ceiling_light"),
            state: state(true, 80, 2700),
            class: DeviceClass::Light,
        });
        reg.upsert(Device {
            id: dev_in("kitchen", "thermometer"),
            state: DeviceState {
                temperature_celsius: Some(21.5),
                ..Default::default()
            },
            class: DeviceClass::Sensor,
        });

        let n = store.save("evening", &reg, None);
        assert_eq!(n, 1, "sensor should not be captured");

        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "ceiling_light"));
    }

    #[test]
    fn get_includes_all_three_state_fields() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries[0].state.on, Some(true));
        assert_eq!(entries[0].state.brightness, Some(80));
        assert_eq!(entries[0].state.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn delete_returns_true_when_present_false_when_missing() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        assert!(!store.delete("evening"));
        store.save("evening", &reg, None);
        assert!(store.delete("evening"));
        assert!(!store.delete("evening"));
    }

    #[test]
    fn delete_removes_from_get_and_exists() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);
        assert!(store.exists("evening"));
        assert!(store.get("evening").is_some());
        store.delete("evening");
        assert!(!store.exists("evening"));
        assert!(store.get("evening").is_none());
    }

    #[test]
    fn delete_uses_canonicalized_name() {
        // "Kitchen Evening" canonicalizes to "kitchen_evening" — same
        // contract as save/get/exists. (See name_canonicalization_collisions.)
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("Kitchen Evening", &reg, None);
        assert!(store.delete("kitchen evening"));
        assert!(!store.exists("Kitchen Evening"));
    }

    #[test]
    fn delete_preserves_other_scenes() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);
        store.save("morning", &reg, None);
        store.save("movie", &reg, None);
        assert!(store.delete("morning"));
        assert_eq!(store.names(), vec!["evening", "movie"]);
    }

    // ------------------------------------------------------------------
    // Persistence tests
    // ------------------------------------------------------------------

    #[test]
    fn persists_and_reloads_scenes_with_state_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenes.json");
        let store = SceneStore::new().with_persistence(path.clone());
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);

        let reloaded = SceneStore::load_from_file(&path)
            .unwrap()
            .with_persistence(path);
        let entries = reloaded.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
        assert_eq!(entries[0].state.on, Some(true));
        assert_eq!(entries[0].state.brightness, Some(80));
        assert_eq!(entries[0].state.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn name_canonicalization_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenes.json");
        let store = SceneStore::new().with_persistence(path.clone());
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("Kitchen Evening", &reg, None);

        let reloaded = SceneStore::load_from_file(&path)
            .unwrap()
            .with_persistence(path);
        assert!(reloaded.exists("kitchen_evening"));
    }

    #[test]
    fn load_from_corrupt_file_yields_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenes.json");
        std::fs::write(&path, b"not json").unwrap();
        let store = SceneStore::load_from_file(&path).unwrap();
        assert!(store.names().is_empty());
    }

    #[test]
    fn write_through_persists_on_save_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenes.json");
        let store = SceneStore::new().with_persistence(path.clone());
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);

        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["scenes"].as_object().unwrap().len(), 1);

        store.delete("evening");
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["scenes"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn malformed_device_id_in_file_is_dropped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scenes.json");
        let raw = serde_json::json!({
            "scenes": {
                "evening": [
                    { "device_id": "not_valid", "state": { "on": true } },
                    { "device_id": "z2m:kitchen/a", "state": { "on": true, "brightness": 80 } }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

        let store = SceneStore::load_from_file(&path).unwrap();
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
    }
}

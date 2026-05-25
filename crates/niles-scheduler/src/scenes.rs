//! Scene store — snapshots and replays device state by name.
//!
//! In-memory only for v0.1. Persistence deferred to a follow-up.

use niles_core::{DeviceId, DeviceRegistry, DeviceState, RoomName};
use std::collections::HashMap;
use std::sync::RwLock;

/// A single device captured in a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneEntry {
    pub device_id: DeviceId,
    pub state: DeviceState,
}

/// In-memory store for named lighting scenes.
///
/// Mirrors the `RwLock<HashMap>` shape of [`ManualModeTracker`].
pub struct SceneStore {
    scenes: RwLock<HashMap<String, Vec<SceneEntry>>>,
}

impl SceneStore {
    pub fn new() -> Self {
        Self {
            scenes: RwLock::new(HashMap::new()),
        }
    }

    /// Snapshot every device in `room` (or the whole registry if `room` is
    /// `None`) and store it under `name`. Returns the number of devices
    /// captured. Overwrites any existing scene with the same canonical name.
    pub fn save(&self, name: &str, registry: &DeviceRegistry, room: Option<&RoomName>) -> usize {
        let key = canonicalize_name(name);
        let entries: Vec<SceneEntry> = match room {
            Some(r) => registry.list_room(r),
            None => registry.list_all(),
        }
        .into_iter()
        .map(|d| SceneEntry {
            device_id: d.id,
            state: d.state,
        })
        .collect();
        let n = entries.len();
        self.scenes_write().insert(key, entries);
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

/// Normalize a raw scene name for use as a HashMap key.
///
/// Rules: trim, lowercase ASCII, collapse runs of ASCII whitespace to `_`.
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
    use niles_core::Device;

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
    fn get_includes_all_three_state_fields() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries[0].state.on, Some(true));
        assert_eq!(entries[0].state.brightness, Some(80));
        assert_eq!(entries[0].state.color_temp_kelvin, Some(2700));
    }
}

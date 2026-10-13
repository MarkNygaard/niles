//! Scene store — snapshots and replays device state by name.
//!
//! Every change is handed to an optional [`DocumentSink`] as a JSON
//! document, which [`SceneStore::from_json`] reads back after a restart.

use niles_core::{DeviceId, DeviceRegistry, DeviceState, RoomName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::persistence::{DocumentSink, hand_over};
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
    pub rgb: Option<[u8; 3]>,
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
            rgb: s.rgb,
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
            rgb: d.rgb,
            temperature_celsius: d.temperature_celsius,
            humidity_percent: d.humidity_percent,
            battery_percent: d.battery_percent,
            // A scene restores lights. Whether a door was open when it
            // was saved is not something a scene can put back, so it is
            // not carried and not re-applied.
            open: None,
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

/// The document form of what the store holds.
fn persistable(inner: &HashMap<String, Vec<SceneEntry>>) -> PersistedScenes {
    let scenes = inner
        .iter()
        .map(|(name, entries)| {
            let entries = entries
                .iter()
                .map(|e| PersistedSceneEntry {
                    device_id: e.device_id.to_string(),
                    state: (&e.state).into(),
                })
                .collect();
            (name.clone(), entries)
        })
        .collect();
    PersistedScenes { scenes }
}

/// The reverse, dropping entries whose device id will not parse: one
/// bad row must not cost somebody every scene they have.
fn rehydrate(persisted: PersistedScenes) -> HashMap<String, Vec<SceneEntry>> {
    let mut scenes = HashMap::new();
    for (name, entries) in persisted.scenes {
        let valid = entries
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
        scenes.insert(canonicalize_name(&name), valid);
    }
    scenes
}

/// In-memory store for named lighting scenes.
///
/// Mirrors the `RwLock<HashMap>` shape of [`ManualModeTracker`].
pub struct SceneStore {
    scenes: RwLock<HashMap<String, Vec<SceneEntry>>>,
    sink: Option<Arc<dyn DocumentSink>>,
}

impl SceneStore {
    pub fn new() -> Self {
        Self {
            scenes: RwLock::new(HashMap::new()),
            sink: None,
        }
    }

    /// Hand every change to `sink`.
    pub fn with_sink(mut self, sink: Arc<dyn DocumentSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Read a document written by [`Self::to_json`].
    pub fn from_json(document: &str) -> serde_json::Result<Self> {
        let persisted: PersistedScenes = serde_json::from_str(document)?;
        Ok(Self {
            scenes: RwLock::new(rehydrate(persisted)),
            sink: None,
        })
    }

    /// Everything held, as the document a sink is given.
    pub fn to_json(&self) -> String {
        let inner = self.scenes_read();
        serde_json::to_string(&persistable(&inner)).unwrap_or_else(|_| "{}".into())
    }

    fn maybe_save(&self, inner: &HashMap<String, Vec<SceneEntry>>) {
        hand_over(self.sink.as_ref(), &persistable(inner), "scenes");
    }

    /// Snapshot every *light* in `room` (or the whole registry if `room`
    /// is `None`) and store it under `name`. Returns the number of
    /// devices captured. Overwrites any existing scene with the same
    /// canonical name.
    ///
    /// Only the house's lights are kept — lights, and the plugs listed
    /// in `lamp_plugs` (see [`niles_core::Device::is_lamp`]). A lamp on
    /// a plug left out of a scene is a lamp the scene cannot turn off;
    /// any other plug put in one would be a fridge a scene could.
    pub fn save(
        &self,
        name: &str,
        registry: &DeviceRegistry,
        room: Option<&RoomName>,
        lamp_plugs: &HashSet<DeviceId>,
    ) -> usize {
        let key = canonicalize_name(name);
        let entries: Vec<SceneEntry> = match room {
            Some(r) => registry.list_room(r),
            None => registry.list_all(),
        }
        .into_iter()
        .filter(|d| d.is_lamp(lamp_plugs))
        .map(|d| SceneEntry {
            // A plug has one thing to be told; whatever else it reports
            // (power draw, say) is not something to put back.
            state: if d.is_light() {
                d.state
            } else {
                DeviceState {
                    on: d.state.on,
                    ..Default::default()
                }
            },
            device_id: d.id,
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
    use crate::persistence::Spy;
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
            r.upsert(Device::new(id, s.clone(), DeviceClass::Light));
        }
        r
    }

    #[test]
    fn save_then_get_roundtrip() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        let n = store.save("evening", &reg, None, &HashSet::new());
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
        let n = store.save("evening", &reg, Some(&kitchen), &HashSet::new());
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
        let n = store.save("all", &reg, None, &HashSet::new());
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
        assert_eq!(store.save("dup", &reg, None, &HashSet::new()), 2);
    }

    #[test]
    fn save_overwrites_by_name() {
        let store = SceneStore::new();
        let reg1 = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        let reg2 = registry_with(&[
            ("kitchen", "a", state(false, 20, 3000)),
            ("bedroom", "b", state(true, 100, 2700)),
        ]);
        store.save("evening", &reg1, None, &HashSet::new());
        let n = store.save("evening", &reg2, None, &HashSet::new());
        assert_eq!(n, 2);
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn exists_lifecycle() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        assert!(!store.exists("evening"));
        store.save("evening", &reg, None, &HashSet::new());
        assert!(store.exists("evening"));
        assert!(!store.exists("morning"));
    }

    #[test]
    fn name_canonicalization_collisions() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("Kitchen Evening", &reg, None, &HashSet::new());
        assert!(store.exists("kitchen evening"));
        assert!(store.exists("  KITCHEN  EVENING  "));
    }

    #[test]
    fn names_returns_sorted() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("b", &reg, None, &HashSet::new());
        store.save("a", &reg, None, &HashSet::new());
        store.save("c", &reg, None, &HashSet::new());
        assert_eq!(store.names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn arc_clones_share_writes() {
        let store = Arc::new(SceneStore::new());
        let store2 = store.clone();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());
        assert!(store2.exists("evening"));
    }

    #[test]
    fn a_lamp_on_a_plug_is_in_the_scene_and_the_fridge_is_not() {
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        let lamp = dev_in("living_room", "corner_lamp");
        reg.upsert(Device::new(
            lamp.clone(),
            DeviceState {
                on: Some(false),
                ..Default::default()
            },
            DeviceClass::Outlet,
        ));
        reg.upsert(Device::new(
            dev_in("kitchen", "fridge"),
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Outlet,
        ));

        let n = store.save("movie", &reg, None, &HashSet::from([lamp.clone()]));
        assert_eq!(n, 1);
        let entries = store.get("movie").unwrap();
        assert_eq!(entries[0].device_id, lamp);
        assert_eq!(entries[0].state.on, Some(false), "off is a state, and kept");
    }

    #[test]
    fn save_drops_non_light_devices() {
        // Sensor-style device: only sensor fields set. Scenes are
        // about lights (ARCHITECTURE.md:491), so it should be filtered
        // by `DeviceClass`, not just by state fields.
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        reg.upsert(Device::new(
            dev_in("kitchen", "ceiling_light"),
            state(true, 80, 2700),
            DeviceClass::Light,
        ));
        reg.upsert(Device::new(
            dev_in("kitchen", "thermometer"),
            DeviceState {
                temperature_celsius: Some(21.5),
                ..Default::default()
            },
            DeviceClass::Sensor,
        ));

        let n = store.save("evening", &reg, None, &HashSet::new());
        assert_eq!(n, 1, "sensor should not be captured");

        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "ceiling_light"));
    }

    #[test]
    fn get_includes_all_three_state_fields() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());
        let entries = store.get("evening").unwrap();
        assert_eq!(entries[0].state.on, Some(true));
        assert_eq!(entries[0].state.brightness, Some(80));
        assert_eq!(entries[0].state.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn get_includes_rgb_field() {
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        reg.upsert(Device::new(
            dev_in("kitchen", "a"),
            DeviceState {
                on: Some(true),
                rgb: Some([255, 128, 0]),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        store.save("evening", &reg, None, &HashSet::new());
        let entries = store.get("evening").unwrap();
        assert_eq!(entries[0].state.rgb, Some([255, 128, 0]));
    }

    #[test]
    fn a_document_survives_a_round_trip_without_a_file() {
        // The database path: no directory, no config section naming
        // one, and the scenes still come back.
        let registry = registry_with(&[("living_room", "bulb_1", state(true, 60, 2700))]);
        let store = SceneStore::new();
        store.save("cosy", &registry, None, &HashSet::new());

        let reloaded = SceneStore::from_json(&store.to_json()).expect("parses");
        assert_eq!(reloaded.names(), vec!["cosy"]);
        let entry = &reloaded.get("cosy").expect("there")[0];
        assert_eq!(entry.state.brightness, Some(60));
        assert_eq!(entry.state.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn a_sink_is_handed_every_change() {
        // Including a delete: a scene somebody removed has to actually
        // disappear, and sending only saves would leave it there until
        // a restart.
        let spy = Arc::new(Spy::default());
        let registry = registry_with(&[("living_room", "bulb_1", state(true, 60, 2700))]);
        let store = SceneStore::new().with_sink(spy.clone());

        store.save("cosy", &registry, None, &HashSet::new());
        store.delete("cosy");

        let seen = spy.documents();
        assert_eq!(seen.len(), 2, "a save and a delete");
        assert!(seen[0].contains("cosy"));
        assert!(!seen[1].contains("cosy"), "the delete has to travel too");
    }

    #[test]
    fn persistence_roundtrips_rgb() {
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        reg.upsert(Device::new(
            dev_in("kitchen", "a"),
            DeviceState {
                on: Some(true),
                rgb: Some([255, 128, 0]),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        store.save("evening", &reg, None, &HashSet::new());

        let loaded = SceneStore::from_json(&store.to_json()).unwrap();
        let entries = loaded.get("evening").unwrap();
        assert_eq!(entries[0].state.rgb, Some([255, 128, 0]));
    }

    #[test]
    fn delete_returns_true_when_present_false_when_missing() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        assert!(!store.delete("evening"));
        store.save("evening", &reg, None, &HashSet::new());
        assert!(store.delete("evening"));
        assert!(!store.delete("evening"));
    }

    #[test]
    fn delete_removes_from_get_and_exists() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());
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
        store.save("Kitchen Evening", &reg, None, &HashSet::new());
        assert!(store.delete("kitchen evening"));
        assert!(!store.exists("Kitchen Evening"));
    }

    #[test]
    fn delete_preserves_other_scenes() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());
        store.save("morning", &reg, None, &HashSet::new());
        store.save("movie", &reg, None, &HashSet::new());
        assert!(store.delete("morning"));
        assert_eq!(store.names(), vec!["evening", "movie"]);
    }

    // ------------------------------------------------------------------
    // Persistence tests
    // ------------------------------------------------------------------

    #[test]
    fn persists_and_reloads_scenes_with_state_fields() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());

        let reloaded = SceneStore::from_json(&store.to_json()).unwrap();
        let entries = reloaded.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
        assert_eq!(entries[0].state.on, Some(true));
        assert_eq!(entries[0].state.brightness, Some(80));
        assert_eq!(entries[0].state.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn name_canonicalization_survives_reload() {
        let store = SceneStore::new();
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("Kitchen Evening", &reg, None, &HashSet::new());

        let reloaded = SceneStore::from_json(&store.to_json()).unwrap();
        assert!(reloaded.exists("kitchen_evening"));
    }

    #[test]
    fn a_document_that_is_not_json_is_refused() {
        // The caller decides what an unreadable document means; the
        // store does not quietly start empty.
        assert!(SceneStore::from_json("not json").is_err());
    }

    #[test]
    fn the_sink_gets_the_whole_set_on_save_and_delete() {
        let spy = Arc::new(Spy::default());
        let store = SceneStore::new().with_sink(spy.clone());
        let reg = registry_with(&[("kitchen", "a", state(true, 80, 2700))]);
        store.save("evening", &reg, None, &HashSet::new());
        assert_eq!(spy.last()["scenes"].as_object().unwrap().len(), 1);

        store.delete("evening");
        assert_eq!(spy.last()["scenes"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn malformed_device_id_in_a_document_is_dropped_not_fatal() {
        let raw = serde_json::json!({
            "scenes": {
                "evening": [
                    { "device_id": "not_valid", "state": { "on": true } },
                    { "device_id": "z2m:kitchen/a", "state": { "on": true, "brightness": 80 } }
                ]
            }
        });

        let store = SceneStore::from_json(&raw.to_string()).unwrap();
        let entries = store.get("evening").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, dev_in("kitchen", "a"));
    }

    #[test]
    fn non_canonical_scene_name_in_a_document_is_canonicalized_on_load() {
        let raw = serde_json::json!({
            "scenes": {
                "Kitchen Evening": [
                    { "device_id": "z2m:kitchen/a", "state": { "on": true } }
                ]
            }
        });

        let store = SceneStore::from_json(&raw.to_string()).unwrap();
        assert!(store.exists("kitchen_evening"));
        assert_eq!(store.names(), vec!["kitchen_evening"]);
    }

    #[test]
    fn empty_scene_survives_reload() {
        let store = SceneStore::new();
        let reg = DeviceRegistry::new();
        reg.upsert(Device::new(
            dev_in("kitchen", "thermometer"),
            DeviceState {
                temperature_celsius: Some(21.5),
                ..Default::default()
            },
            DeviceClass::Sensor,
        ));
        store.save("sensors_only", &reg, None, &HashSet::new());
        assert!(store.exists("sensors_only"));
        assert_eq!(store.get("sensors_only").unwrap().len(), 0);

        let reloaded = SceneStore::from_json(&store.to_json()).unwrap();
        assert!(reloaded.exists("sensors_only"));
        assert_eq!(reloaded.get("sensors_only").unwrap().len(), 0);
    }
}

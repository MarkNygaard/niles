//! In-memory device registry.
//!
//! Derived from upstream device sources at runtime per the no-UI
//! strategy in the architecture spec. Not a persistent database —
//! the registry is rebuilt from the source on restart.

use crate::device::{Device, DeviceId, DeviceState, RoomName};
use std::collections::HashMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

const POISONED: &str = "no thread panics holding the device map";

/// In-memory registry keyed by `DeviceId`.
///
/// Wrap in `Arc<DeviceRegistry>` to share between subsystems.
#[derive(Default)]
pub struct DeviceRegistry {
    devices: RwLock<HashMap<DeviceId, Device>>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a device.
    ///
    /// Having reported an action survives the replacement: it is a fact
    /// about the device, and sources re-describe their devices on every
    /// reconnect without knowing it.
    pub fn upsert(&self, mut device: Device) {
        let mut guard = self.write();
        if let Some(existing) = guard.get(&device.id) {
            device.reports_actions |= existing.reports_actions;
        }
        guard.insert(device.id.clone(), device);
    }

    /// Note that a device reported an action. Returns true when it had
    /// not before, so a source that remembers it knows to.
    pub fn note_action(&self, id: &DeviceId) -> bool {
        let mut guard = self.write();
        match guard.get_mut(id) {
            Some(device) if !device.reports_actions => {
                device.reports_actions = true;
                true
            }
            _ => false,
        }
    }

    /// Remove a device. Returns the removed entry if any.
    pub fn remove(&self, id: &DeviceId) -> Option<Device> {
        let mut guard = self.write();
        guard.remove(id)
    }

    /// Get a snapshot of a device. The lock is not held across the caller.
    pub fn get(&self, id: &DeviceId) -> Option<Device> {
        let guard = self.read();
        guard.get(id).cloned()
    }

    /// Record whether the source can reach a device.
    ///
    /// Returns true when this changed something, so a caller can skip
    /// publishing an event nobody needs — availability messages are
    /// retained and arrive again on every reconnect.
    pub fn set_available(&self, id: &DeviceId, available: bool) -> bool {
        let mut guard = self.write();
        let Some(device) = guard.get_mut(id) else {
            return false;
        };
        if device.available == available {
            return false;
        }
        device.available = available;
        true
    }

    /// Merge a partial state update into an existing device.
    ///
    /// `Some` fields in `partial` overwrite the stored value; `None`
    /// fields preserve it. This matches the `DeviceState` contract:
    /// upstream reports only what changed, so a `None` means "not
    /// reported", never "cleared". Returns `true` if the device
    /// existed and was updated.
    ///
    /// Readings merge name by name for the same reason: a payload
    /// carrying `power` must not wipe `energy`.
    pub fn merge_state(&self, id: &DeviceId, partial: DeviceState) -> bool {
        let mut guard = self.write();
        let Some(device) = guard.get_mut(id) else {
            return false;
        };
        device.state.merge(partial);
        true
    }

    /// All devices in the given room.
    pub fn list_room(&self, room: &RoomName) -> Vec<Device> {
        let guard = self.read();
        guard
            .values()
            .filter(|d| d.id.room() == room)
            .cloned()
            .collect()
    }

    /// All devices, regardless of room.
    pub fn list_all(&self) -> Vec<Device> {
        let guard = self.read();
        guard.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<DeviceId, Device>> {
        self.devices.read().expect(POISONED)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<DeviceId, Device>> {
        self.devices.write().expect(POISONED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceClass, DeviceName, RoomName};

    fn make_device(room: &str, name: &str) -> Device {
        Device::new(
            DeviceId::new(
                "z2m",
                RoomName::parse(room).unwrap(),
                DeviceName::parse(name).unwrap(),
            )
            .unwrap(),
            DeviceState::default(),
            DeviceClass::Unknown,
        )
    }

    #[test]
    fn upsert_then_get() {
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());

        let retrieved = registry.get(&device.id).unwrap();
        assert_eq!(retrieved.id, device.id);
    }

    #[test]
    fn upsert_replaces() {
        let registry = DeviceRegistry::new();
        let mut device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());

        device.state.on = Some(true);
        registry.upsert(device.clone());

        let retrieved = registry.get(&device.id).unwrap();
        assert_eq!(retrieved.state.on, Some(true));
    }

    #[test]
    fn an_action_is_noted_once_and_survives_the_device_being_described_again() {
        let registry = DeviceRegistry::new();
        let device = make_device("office", "printer");
        registry.upsert(device.clone());
        assert!(registry.note_action(&device.id));
        assert!(!registry.note_action(&device.id), "already known");
        registry.upsert(device.clone());
        assert!(registry.get(&device.id).unwrap().reports_actions);
        assert!(!registry.note_action(&make_device("office", "gone").id));
    }

    #[test]
    fn merge_state_existing() {
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());

        let partial = DeviceState {
            on: Some(true),
            brightness: Some(80),
            ..Default::default()
        };
        assert!(registry.merge_state(&device.id, partial.clone()));

        let retrieved = registry.get(&device.id).unwrap();
        assert_eq!(retrieved.state, partial);
    }

    /// The contract on `DeviceState`: `None` in a partial means "not
    /// reported", not "cleared". Subsequent partials must not clobber
    /// fields that earlier reports set.
    #[test]
    fn merge_state_preserves_unreported_fields() {
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());

        // Full initial state.
        registry.merge_state(
            &device.id,
            DeviceState {
                on: Some(true),
                brightness: Some(100),
                color_temp_kelvin: Some(4000),
                ..Default::default()
            },
        );

        // Z2M sends a brightness-only delta.
        registry.merge_state(
            &device.id,
            DeviceState {
                brightness: Some(60),
                ..Default::default()
            },
        );

        let s = registry.get(&device.id).unwrap().state;
        assert_eq!(s.on, Some(true), "on must survive a brightness-only update");
        assert_eq!(s.brightness, Some(60));
        assert_eq!(
            s.color_temp_kelvin,
            Some(4000),
            "color_temp must survive a brightness-only update"
        );
    }
    /// RGB must survive a brightness-only delta, and a new RGB must
    /// be applied when present.
    #[test]
    fn merge_state_preserves_and_updates_rgb() {
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());

        // Initial state with RGB.
        registry.merge_state(
            &device.id,
            DeviceState {
                on: Some(true),
                rgb: Some([255, 128, 0]),
                ..Default::default()
            },
        );

        // Brightness-only delta must keep RGB.
        registry.merge_state(
            &device.id,
            DeviceState {
                brightness: Some(50),
                ..Default::default()
            },
        );

        let s = registry.get(&device.id).unwrap().state;
        assert_eq!(s.brightness, Some(50));
        assert_eq!(
            s.rgb,
            Some([255, 128, 0]),
            "rgb must survive a brightness-only update"
        );

        // New RGB must overwrite old RGB.
        registry.merge_state(
            &device.id,
            DeviceState {
                rgb: Some([0, 255, 0]),
                ..Default::default()
            },
        );

        let s = registry.get(&device.id).unwrap().state;
        assert_eq!(
            s.rgb,
            Some([0, 255, 0]),
            "rgb must be overwritten by a new rgb"
        );
    }

    #[test]
    fn readings_merge_name_by_name() {
        use crate::device::Reading;
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "dishwasher_plug");
        registry.upsert(device.clone());

        let reading = |name: &str, value: f64, unit: &str| DeviceState {
            readings: [(name.to_string(), Reading::new(value, Some(unit.into())))].into(),
            ..Default::default()
        };
        registry.merge_state(&device.id, reading("energy", 12.4, "kWh"));
        registry.merge_state(&device.id, reading("power", 340.0, "W"));
        registry.merge_state(&device.id, reading("power", 5.0, "W"));

        let readings = registry.get(&device.id).unwrap().state.readings;
        assert_eq!(readings["energy"].value, 12.4, "power must not wipe energy");
        assert_eq!(readings["power"].value, 5.0, "a new value replaces the old");
        assert_eq!(readings["power"].unit.as_deref(), Some("W"));
    }

    #[test]
    fn merge_state_missing_returns_false() {
        let registry = DeviceRegistry::new();
        let id = make_device("kitchen", "ceiling_light").id;
        assert!(!registry.merge_state(&id, DeviceState::default()));
    }

    #[test]
    fn list_room_filters() {
        let registry = DeviceRegistry::new();
        registry.upsert(make_device("kitchen", "ceiling_light"));
        registry.upsert(make_device("kitchen", "counter_light"));
        registry.upsert(make_device("living_room", "floor_lamp"));

        let kitchen = RoomName::parse("kitchen").unwrap();
        assert_eq!(registry.list_room(&kitchen).len(), 2);
        assert_eq!(registry.list_all().len(), 3);
    }

    #[test]
    fn remove() {
        let registry = DeviceRegistry::new();
        let device = make_device("kitchen", "ceiling_light");
        registry.upsert(device.clone());
        assert!(registry.remove(&device.id).is_some());
        assert!(registry.get(&device.id).is_none());
        assert!(registry.is_empty());
    }
}

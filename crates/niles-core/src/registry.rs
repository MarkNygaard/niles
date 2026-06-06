//! In-memory device registry.
//!
//! Derived from upstream device sources at runtime per the no-UI
//! strategy in the architecture spec. Not a persistent database —
//! the registry is rebuilt from the source on restart.

use crate::device::{Device, DeviceId, DeviceState, RoomName};
use std::collections::HashMap;
use std::sync::RwLock;

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
    pub fn upsert(&self, device: Device) {
        let mut guard = self.devices.write().unwrap();
        guard.insert(device.id.clone(), device);
    }

    /// Remove a device. Returns the removed entry if any.
    pub fn remove(&self, id: &DeviceId) -> Option<Device> {
        let mut guard = self.devices.write().unwrap();
        guard.remove(id)
    }

    /// Get a snapshot of a device. The lock is not held across the caller.
    pub fn get(&self, id: &DeviceId) -> Option<Device> {
        let guard = self.devices.read().unwrap();
        guard.get(id).cloned()
    }

    /// Merge a partial state update into an existing device.
    ///
    /// `Some` fields in `partial` overwrite the stored value; `None`
    /// fields preserve it. This matches the `DeviceState` contract:
    /// upstream reports only what changed, so a `None` means "not
    /// reported", never "cleared". Returns `true` if the device
    /// existed and was updated.
    pub fn merge_state(&self, id: &DeviceId, partial: DeviceState) -> bool {
        let mut guard = self.devices.write().unwrap();
        let Some(device) = guard.get_mut(id) else {
            return false;
        };
        let s = &mut device.state;
        s.on = partial.on.or(s.on);
        s.brightness = partial.brightness.or(s.brightness);
        s.color_temp_kelvin = partial.color_temp_kelvin.or(s.color_temp_kelvin);
        s.rgb = partial.rgb.or(s.rgb);
        s.temperature_celsius = partial.temperature_celsius.or(s.temperature_celsius);
        s.humidity_percent = partial.humidity_percent.or(s.humidity_percent);
        s.battery_percent = partial.battery_percent.or(s.battery_percent);
        true
    }

    /// All devices in the given room.
    pub fn list_room(&self, room: &RoomName) -> Vec<Device> {
        let guard = self.devices.read().unwrap();
        guard
            .values()
            .filter(|d| d.id.room() == room)
            .cloned()
            .collect()
    }

    /// All devices, regardless of room.
    pub fn list_all(&self) -> Vec<Device> {
        let guard = self.devices.read().unwrap();
        guard.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.devices.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.read().unwrap().is_empty()
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

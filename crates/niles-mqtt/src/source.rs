//! Z2M device source — consumes MQTT messages and populates a
//! `niles_core::DeviceRegistry` while publishing change events on
//! the bus.
//!
//! Topic conventions (with default `prefix = "zigbee2mqtt"`):
//! - `zigbee2mqtt/bridge/devices` — full device list (JSON array of
//!   [`Z2mDevice`]). Republished by Z2M whenever the inventory changes.
//! - `zigbee2mqtt/<room>/<device>` — per-device state JSON.
//!
//! Anything else under `<prefix>/...` (e.g. `bridge/logging`,
//! `bridge/info`) is ignored.

use crate::client::{Message, MqttClient};
use crate::error::Result;
use crate::z2m::{parse_device_list, parse_state};
use niles_core::{DeviceId, DeviceRegistry, Event, EventBus};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

/// Consumes Z2M MQTT messages and keeps a shared `DeviceRegistry`
/// in sync, publishing change events on the bus.
pub struct Z2mSource {
    client: MqttClient,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    prefix: String,
}

impl Z2mSource {
    /// Wrap a connected `MqttClient` to drive the given registry/bus.
    /// `prefix` is the Z2M topic root (typically `"zigbee2mqtt"`,
    /// without trailing slash).
    pub fn new(
        client: MqttClient,
        registry: Arc<DeviceRegistry>,
        bus: EventBus,
        prefix: impl Into<String>,
    ) -> Self {
        Self {
            client,
            registry,
            bus,
            prefix: prefix.into(),
        }
    }

    /// Subscribe to the Z2M topics and run the message loop until the
    /// underlying MQTT client disconnects.
    pub async fn run(mut self) -> Result<()> {
        let bridge_topic = format!("{}/bridge/devices", self.prefix);
        // `+/+` matches `<room>/<device>` per-device state topics.
        let state_pattern = format!("{}/+/+", self.prefix);
        self.client.subscribe(&bridge_topic).await?;
        self.client.subscribe(&state_pattern).await?;

        while let Some(msg) = self.client.next_message().await {
            self.dispatch(&msg);
        }
        Ok(())
    }

    /// Route an incoming message to the right handler.
    pub(crate) fn dispatch(&self, msg: &Message) {
        let Some(rest) = msg.topic.strip_prefix(&self.prefix) else {
            return;
        };
        let Some(rest) = rest.strip_prefix('/') else {
            return;
        };

        if rest == "bridge/devices" {
            handle_device_list(&msg.payload, &self.registry, &self.bus);
        } else if let Some((room, device)) = split_room_device(rest) {
            // Skip Z2M's internal `bridge/*` topics other than `bridge/devices`.
            if room == "bridge" {
                return;
            }
            handle_device_state(room, device, &msg.payload, &self.registry, &self.bus);
        }
    }
}

/// Split `"<room>/<device>"`. Returns `None` if there's no slash or
/// the form is more complex (e.g. `bridge/logging/error`).
fn split_room_device(s: &str) -> Option<(&str, &str)> {
    let (room, rest) = s.split_once('/')?;
    if rest.contains('/') {
        // Nested path — not a top-level device.
        return None;
    }
    Some((room, rest))
}

/// Parse a `bridge/devices` payload and reconcile the registry: add
/// new devices, update friendly_name renames (handled implicitly by
/// the registry keying), remove devices no longer in the list.
pub(crate) fn handle_device_list(payload: &[u8], registry: &DeviceRegistry, bus: &EventBus) {
    let devices = match parse_device_list(payload) {
        Ok(d) => d,
        Err(e) => {
            warn!("failed to parse bridge/devices payload: {e}");
            return;
        }
    };

    // Build the new set of IDs we want present.
    let mut new_ids: HashSet<DeviceId> = HashSet::new();
    for z2m in &devices {
        if !z2m.is_user_device() {
            continue;
        }
        match z2m.to_device() {
            Ok(device) => {
                let id = device.id.clone();
                new_ids.insert(id);
                let existed = registry.get(&device.id).is_some();
                registry.upsert(device.clone());
                if !existed {
                    debug!("discovered {}", device.id);
                    bus.publish(Event::DeviceAdded { device });
                }
            }
            Err(e) => {
                warn!(
                    "skipping Z2M device with friendly_name {:?}: {e}",
                    z2m.friendly_name
                );
            }
        }
    }

    // Remove devices that disappeared from the source. We only touch
    // devices whose id.source() matches us (`"z2m"`) so other sources
    // (Shelly, Matter, …) aren't affected when they exist.
    let to_remove: Vec<DeviceId> = registry
        .list_all()
        .into_iter()
        .filter(|d| d.id.source() == "z2m" && !new_ids.contains(&d.id))
        .map(|d| d.id)
        .collect();
    for id in to_remove {
        registry.remove(&id);
        debug!("removed {}", id);
        bus.publish(Event::DeviceRemoved { id });
    }
}

/// Parse a per-device state payload and update the registry. Emits a
/// `DeviceStateChanged` event regardless of whether the device was
/// already known — the source of truth is the bridge/devices list,
/// and state may arrive before the device list in startup race
/// conditions.
pub(crate) fn handle_device_state(
    room: &str,
    device: &str,
    payload: &[u8],
    registry: &DeviceRegistry,
    bus: &EventBus,
) {
    let id_str = format!("z2m:{room}/{device}");
    let id = match DeviceId::parse(&id_str) {
        Ok(id) => id,
        Err(e) => {
            debug!("ignoring state topic {id_str:?}: {e}");
            return;
        }
    };
    let z2m_state = match parse_state(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!("failed to parse state payload for {id}: {e}");
            return;
        }
    };
    let state = z2m_state.to_device_state();
    let updated = registry.update_state(&id, state.clone());
    if !updated {
        debug!("state for {id} arrived before bridge/devices — buffering");
        // We still publish the event so subscribers see it; downstream
        // can decide whether to ignore states for unknown devices.
    }
    bus.publish(Event::DeviceStateChanged { id, state });
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::DeviceRegistry;

    fn fixtures() -> (
        Arc<DeviceRegistry>,
        EventBus,
        tokio::sync::broadcast::Receiver<Event>,
    ) {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let rx = bus.subscribe();
        (registry, bus, rx)
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    // ---- handle_device_list --------------------------------------

    #[test]
    fn populates_registry_from_device_list() {
        let (registry, bus, mut rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"},
            {"ieee_address":"0x2","friendly_name":"office/desk_lamp","type":"EndDevice"},
            {"ieee_address":"0x3","friendly_name":"Coordinator","type":"Coordinator"}
        ]"#;
        handle_device_list(payload, &registry, &bus);

        let devices = registry.list_all();
        assert_eq!(devices.len(), 2, "coordinator must not be in registry");

        let events = drain(&mut rx);
        assert_eq!(events.len(), 2, "two DeviceAdded events expected");
        for ev in events {
            assert!(matches!(ev, Event::DeviceAdded { .. }));
        }
    }

    #[test]
    fn second_device_list_removes_disappeared_devices() {
        let (registry, bus, mut rx) = fixtures();

        let first = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"},
            {"ieee_address":"0x2","friendly_name":"office/desk_lamp","type":"EndDevice"}
        ]"#;
        handle_device_list(first, &registry, &bus);
        drain(&mut rx); // discard initial add events

        let second = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(second, &registry, &bus);

        assert_eq!(registry.list_all().len(), 1, "office device should be gone");
        let events = drain(&mut rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::DeviceRemoved { .. })),
            "expected a DeviceRemoved event"
        );
    }

    #[test]
    fn rediscovering_same_device_does_not_re_emit_added() {
        let (registry, bus, mut rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(payload, &registry, &bus);
        drain(&mut rx); // first add
        handle_device_list(payload, &registry, &bus);
        let events = drain(&mut rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::DeviceAdded { .. })),
            "rediscovery should not re-emit DeviceAdded"
        );
    }

    #[test]
    fn ignores_devices_with_invalid_friendly_name() {
        let (registry, bus, _rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"Bad Name With Spaces","type":"Router"}
        ]"#;
        handle_device_list(payload, &registry, &bus);
        assert!(registry.is_empty());
    }

    #[test]
    fn malformed_device_list_payload_is_logged_not_panicked() {
        let (registry, bus, _rx) = fixtures();
        handle_device_list(b"not json", &registry, &bus);
        assert!(registry.is_empty());
    }

    // ---- handle_device_state -------------------------------------

    #[test]
    fn state_message_updates_registry_and_emits_event() {
        let (registry, bus, mut rx) = fixtures();
        let device_list = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(device_list, &registry, &bus);
        drain(&mut rx);

        let state_payload = br#"{"state":"ON","brightness":254,"color_temp":250}"#;
        handle_device_state("kitchen", "ceiling_light", state_payload, &registry, &bus);

        let id = DeviceId::parse("z2m:kitchen/ceiling_light").unwrap();
        let dev = registry.get(&id).unwrap();
        assert_eq!(dev.state.on, Some(true));
        assert_eq!(dev.state.brightness, Some(100));
        assert_eq!(dev.state.color_temp_kelvin, Some(4000));

        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::DeviceStateChanged { .. }));
    }

    #[test]
    fn state_for_unknown_device_still_emits_event() {
        // State can arrive before bridge/devices on startup; the
        // event still flows so downstream can decide what to do.
        let (registry, bus, mut rx) = fixtures();
        let payload = br#"{"state":"ON"}"#;
        handle_device_state("kitchen", "ceiling_light", payload, &registry, &bus);
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::DeviceStateChanged { .. }));
    }

    #[test]
    fn state_with_invalid_topic_segment_is_ignored() {
        let (registry, bus, mut rx) = fixtures();
        handle_device_state("Kitchen", "ceiling_light", b"{}", &registry, &bus);
        assert!(drain(&mut rx).is_empty(), "no event for invalid room name");
    }

    // ---- dispatch routing ----------------------------------------

    fn make_source(prefix: &str) -> (Arc<DeviceRegistry>, EventBus, Z2mSource) {
        // We can't easily construct an MqttClient without a runtime
        // and a broker, but dispatch only uses prefix/registry/bus.
        // So we build a "dummy" source by tunnelling through
        // private fields — done via a tokio runtime that connects to
        // a nonsense host. The pump task exits immediately on error;
        // dispatch is callable as long as the struct exists.
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let opts = crate::MqttOptions::new("127.0.0.1", 1, "test").with_credentials("u", "p");
        let client = MqttClient::connect(opts);
        let source = Z2mSource::new(client, registry.clone(), bus.clone(), prefix);
        (registry, bus, source)
    }

    #[tokio::test]
    async fn dispatch_routes_bridge_devices() {
        let (registry, _bus, source) = make_source("zigbee2mqtt");
        let msg = Message {
            topic: "zigbee2mqtt/bridge/devices".into(),
            payload: br#"[
                {"ieee_address":"0x1","friendly_name":"office/desk_lamp","type":"EndDevice"}
            ]"#
            .to_vec(),
        };
        source.dispatch(&msg);
        assert_eq!(registry.list_all().len(), 1);
    }

    #[tokio::test]
    async fn dispatch_routes_state_topics() {
        let (registry, _bus, source) = make_source("zigbee2mqtt");
        let msg = Message {
            topic: "zigbee2mqtt/office/desk_lamp".into(),
            payload: br#"{"state":"ON"}"#.to_vec(),
        };
        source.dispatch(&msg);
        // No registry entry yet (no bridge/devices first), but the
        // call should not panic and the bus event should have fired.
        let id = DeviceId::parse("z2m:office/desk_lamp").unwrap();
        assert!(registry.get(&id).is_none());
    }

    #[tokio::test]
    async fn dispatch_ignores_unrelated_topics() {
        let (registry, _bus, source) = make_source("zigbee2mqtt");
        let msg = Message {
            topic: "homeassistant/light/foo".into(),
            payload: b"{}".to_vec(),
        };
        source.dispatch(&msg);
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn dispatch_ignores_z2m_internal_topics() {
        let (registry, _bus, source) = make_source("zigbee2mqtt");
        let msg = Message {
            topic: "zigbee2mqtt/bridge/logging".into(),
            payload: br#"{"level":"info"}"#.to_vec(),
        };
        source.dispatch(&msg);
        assert!(registry.is_empty());
    }
}

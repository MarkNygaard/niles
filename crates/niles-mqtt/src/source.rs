//! Z2M device source — consumes MQTT messages and populates a
//! `niles_core::DeviceRegistry` while publishing change events on
//! the bus.
//!
//! Topic conventions (with default `prefix = "zigbee2mqtt"`):
//! - `zigbee2mqtt/bridge/devices` — full device list (JSON array of
//!   [`Z2mDevice`]). Republished by Z2M whenever the inventory changes.
//! - `zigbee2mqtt/<room>/<device>` — per-device state JSON.
//! - `zigbee2mqtt/<room>/<device>/action` — per-device action strings
//!   from button / dimmer devices.
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
        // `+/+` matches every two-level topic under the prefix, which
        // covers both `bridge/devices` (the device list) and
        // `<room>/<device>` (per-device state). Routing inside
        // `dispatch` distinguishes them.
        let state_pattern = format!("{}/+/+", self.prefix);
        self.client.subscribe(&state_pattern).await?;
        let action_pattern = format!("{}/+/+/action", self.prefix);
        self.client.subscribe(&action_pattern).await?;

        while let Some(msg) = self.client.next_message().await {
            dispatch(&msg, &self.prefix, &self.registry, &self.bus);
        }
        Ok(())
    }
}

/// Route an incoming message to the right handler. Extracted as a
/// free function so tests can exercise routing without constructing a
/// real `MqttClient`.
pub(crate) fn dispatch(msg: &Message, prefix: &str, registry: &DeviceRegistry, bus: &EventBus) {
    let Some(rest) = msg.topic.strip_prefix(prefix) else {
        return;
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return;
    };

    if rest == "bridge/devices" {
        handle_device_list(&msg.payload, registry, bus);
    } else if let Some(action_rest) = rest.strip_suffix("/action") {
        if let Some((room, device)) = split_room_device(action_rest) {
            if room == "bridge" {
                return;
            }
            handle_device_action(room, device, &msg.payload, bus);
        }
    } else if let Some((room, device)) = split_room_device(rest) {
        // Skip Z2M's internal `bridge/*` topics other than `bridge/devices`.
        if room == "bridge" {
            return;
        }
        // Skip Z2M's per-device subtopics. For `room/device` friendly_names
        // these are 3 levels deep and wouldn't match our `+/+` subscription;
        // but for *flat* friendly_names (e.g. `bathroom_sensor_motion`) the
        // form `<flat>/availability` is 2 levels and *does* match. Without
        // this guard we'd happily treat `availability` as a device name
        // and emit bogus `DeviceStateChanged` events. Z2M's reserved
        // subtopics for the friendly-name prefix:
        //   <name>/availability   — online/offline tracking
        //   <name>/set            — write commands (often echoed back)
        //   <name>/get            — request-state messages
        if matches!(device, "availability" | "set" | "get") {
            return;
        }
        handle_device_state(room, device, &msg.payload, registry, bus);
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

/// Maximum action-payload size we'll accept. Z2M action strings are
/// short (`up_hold_release` is the longest at 16 bytes); anything
/// orders of magnitude larger is almost certainly a misconfigured
/// retain or a wrong-topic publish. We log and drop.
const MAX_ACTION_PAYLOAD: usize = 256;

/// Parse a per-device action payload (plain UTF-8 string) and emit a
/// `DeviceAction` event. Drops non-UTF-8 / oversize payloads with a warn.
pub(crate) fn handle_device_action(room: &str, device: &str, payload: &[u8], bus: &EventBus) {
    if payload.len() >= MAX_ACTION_PAYLOAD {
        let len = payload.len();
        warn!(
            "action payload for {room}/{device} is {len} bytes (>= {MAX_ACTION_PAYLOAD}); dropping"
        );
        return;
    }
    let action = match std::str::from_utf8(payload) {
        Ok(s) => s.to_string(),
        Err(e) => {
            warn!("action payload for {room}/{device} is not UTF-8: {e}");
            return;
        }
    };
    let id_str = format!("z2m:{room}/{device}");
    let id = match DeviceId::parse(&id_str) {
        Ok(id) => id,
        Err(e) => {
            debug!("ignoring action topic {id_str:?}: {e}");
            return;
        }
    };
    bus.publish(Event::DeviceAction { id, action });
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
    if !z2m_state.has_actionable_state_field() {
        // Dimmer-style devices republish `{"action":..,"battery":..,
        // "linkquality":..}` on every press. None of those are tracked
        // state in this PR, so skip the merge + event entirely.
        debug!("skipping state payload for {id} (no actionable field)");
        return;
    }
    let partial = z2m_state.to_device_state();
    if !registry.merge_state(&id, partial.clone()) {
        // State can arrive before bridge/devices on startup. We
        // discard it from the registry (no entry to merge into) but
        // still publish the event so any pre-bound subscribers see
        // it. Z2M republishes the full inventory shortly after
        // connect, which will re-prime the registry; devices report
        // current state on the next change.
        debug!("state for unknown {id} discarded; awaiting bridge/devices");
    }
    bus.publish(Event::DeviceStateChanged { id, state: partial });
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

    /// Z2M only republishes the fields that changed. A subsequent
    /// delta must not clobber fields the registry already knows about.
    #[test]
    fn partial_state_messages_do_not_clobber_known_fields() {
        let (registry, bus, mut rx) = fixtures();
        let device_list = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(device_list, &registry, &bus);
        drain(&mut rx);

        // Full state arrives first.
        handle_device_state(
            "kitchen",
            "ceiling_light",
            br#"{"state":"ON","brightness":254,"color_temp":250}"#,
            &registry,
            &bus,
        );
        // Then a brightness-only delta.
        handle_device_state(
            "kitchen",
            "ceiling_light",
            br#"{"brightness":127}"#,
            &registry,
            &bus,
        );

        let id = DeviceId::parse("z2m:kitchen/ceiling_light").unwrap();
        let s = registry.get(&id).unwrap().state;
        assert_eq!(s.on, Some(true), "on must survive a brightness-only update");
        assert_eq!(s.brightness, Some(50));
        assert_eq!(
            s.color_temp_kelvin,
            Some(4000),
            "color_temp must survive a brightness-only update"
        );
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

    #[test]
    fn dispatch_routes_bridge_devices() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/bridge/devices".into(),
            payload: br#"[
                {"ieee_address":"0x1","friendly_name":"office/desk_lamp","type":"EndDevice"}
            ]"#
            .to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        assert_eq!(registry.list_all().len(), 1);
    }

    #[test]
    fn dispatch_routes_state_topics() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/office/desk_lamp".into(),
            payload: br#"{"state":"ON"}"#.to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        // No registry entry yet (no bridge/devices first), but the
        // call should not panic and the bus event should have fired.
        let id = DeviceId::parse("z2m:office/desk_lamp").unwrap();
        assert!(registry.get(&id).is_none());
    }

    #[test]
    fn dispatch_ignores_z2m_per_device_subtopics() {
        // A flat-named device (e.g. one paired before adopting the
        // <room>/<device> convention, or one whose retained
        // availability message lingers after removal) would
        // otherwise produce bogus DeviceStateChanged events for
        // "devices" called "availability", "set", or "get".
        let (registry, bus, mut rx) = fixtures();
        for sub in ["availability", "set", "get"] {
            let topic = format!("zigbee2mqtt/bathroom_sensor_motion/{sub}");
            let msg = Message {
                topic,
                payload: br#"{"state":"online"}"#.to_vec(),
            };
            dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        }
        // No events emitted, no registry mutation:
        assert!(registry.is_empty());
        assert!(rx.try_recv().is_err(), "no event should have fired");
    }

    #[test]
    fn dispatch_ignores_unrelated_topics() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "homeassistant/light/foo".into(),
            payload: b"{}".to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        assert!(registry.is_empty());
    }

    #[test]
    fn dispatch_ignores_z2m_internal_topics() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/bridge/logging".into(),
            payload: br#"{"level":"info"}"#.to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        assert!(registry.is_empty());
    }

    // ---- handle_device_action ------------------------------------

    #[test]
    fn action_message_emits_device_action_event() {
        let (_registry, bus, mut rx) = fixtures();
        handle_device_action("office", "switch", b"on_press", &bus);
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::DeviceAction { id, action } => {
                assert_eq!(id, &DeviceId::parse("z2m:office/switch").unwrap());
                assert_eq!(action, "on_press");
            }
            _ => panic!("expected DeviceAction"),
        }
    }

    #[test]
    fn action_with_invalid_topic_segment_is_dropped() {
        let (_registry, bus, mut rx) = fixtures();
        // Uppercase room is invalid per RoomName parsing rules.
        handle_device_action("Office", "switch", b"on_press", &bus);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn action_with_oversize_payload_is_dropped() {
        let (_registry, bus, mut rx) = fixtures();
        let payload = vec![b'a'; 256];
        handle_device_action("office", "switch", &payload, &bus);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn action_with_non_utf8_payload_is_dropped() {
        let (_registry, bus, mut rx) = fixtures();
        handle_device_action("office", "switch", &[0xff, 0xfe], &bus);
        assert!(drain(&mut rx).is_empty());
    }

    // ---- dispatch routing for action topics ---------------------

    #[test]
    fn dispatch_routes_action_topics() {
        let (registry, bus, mut rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/office/switch/action".into(),
            payload: b"on_press".to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::DeviceAction { .. }));
    }

    #[test]
    fn dispatch_action_misshapen_topic_is_dropped() {
        let (registry, bus, mut rx) = fixtures();
        // 4 segments under prefix (room/device/sub/action) — not our shape.
        let msg = Message {
            topic: "zigbee2mqtt/office/switch/extra/action".into(),
            payload: b"on_press".to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn dispatch_action_under_bridge_is_dropped() {
        // Hypothetical `bridge/<name>/action` from a misconfigured
        // Z2M bridge — never our intent. The state path filters
        // `bridge` as a room; the action path must do the same.
        let (registry, bus, mut rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/bridge/devices/action".into(),
            payload: b"on_press".to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus);
        assert!(drain(&mut rx).is_empty());
    }

    // ---- JSON state filter --------------------------------------

    #[test]
    fn state_with_only_action_field_does_not_emit() {
        let (registry, bus, mut rx) = fixtures();
        handle_device_state(
            "office",
            "switch",
            br#"{"action":"on_press","battery":100,"linkquality":168}"#,
            &registry,
            &bus,
        );
        assert!(
            drain(&mut rx).is_empty(),
            "action-only payload must not emit DeviceStateChanged"
        );
    }
}

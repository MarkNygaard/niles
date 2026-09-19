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
use crate::z2m::{Z2mGroup, parse_device_list, parse_group_list, parse_state};
use niles_core::{DeviceId, DeviceRegistry, Event, EventBus, LightCapabilities};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::{debug, warn};

/// Consumes Z2M MQTT messages and keeps a shared `DeviceRegistry`
/// in sync, publishing change events on the bus.
pub struct Z2mSource {
    client: MqttClient,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    prefix: String,
    /// What `bridge/groups` and `bridge/devices` know between them.
    index: Mutex<GroupIndex>,
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
            index: Mutex::new(GroupIndex::default()),
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
        // Z2M publishes this per device when availability tracking is
        // on. Without it a light that has been unplugged for a week
        // still reads as on at 100%, because the last state it ever
        // published is the last state anybody ever hears.
        let availability_pattern = format!("{}/+/+/availability", self.prefix);
        self.client.subscribe(&availability_pattern).await?;

        let publisher = self.client.publisher();
        while let Some(msg) = self.client.next_message().await {
            for id in dispatch(&msg, &self.prefix, &self.registry, &self.bus, &self.index) {
                request_state(&publisher, &self.prefix, &id).await;
            }
        }
        Ok(())
    }
}

/// What the two bridge lists know between them.
///
/// Groups and devices arrive on separate topics, in either order, and
/// neither is enough on its own: a group has no `definition`, so what
/// it can be told has to come from its members, and a member is only
/// identifiable by an ieee address that the device list translates.
/// So both are remembered and the registry is rebuilt from the pair
/// whenever either changes.
#[derive(Default)]
pub(crate) struct GroupIndex {
    groups: Vec<Z2mGroup>,
    /// ieee address → the id that device was given.
    ids: HashMap<String, DeviceId>,
    /// ieee address → what it can be told, for the union a group takes.
    caps: HashMap<String, LightCapabilities>,
}

impl GroupIndex {
    /// The ieee addresses that belong to some group, and so belong to
    /// no room list of their own.
    fn members(&self) -> HashSet<&str> {
        self.groups
            .iter()
            .flat_map(|g| g.members.iter())
            .map(|m| m.ieee_address.as_str())
            .collect()
    }

    /// The ids the groups themselves occupy.
    fn group_ids(&self) -> Vec<DeviceId> {
        self.groups
            .iter()
            .filter_map(|g| DeviceId::parse(&format!("z2m:{}", g.friendly_name)).ok())
            .collect()
    }
}

/// Put the groups into the registry and take their members out.
///
/// Both halves matter. A group that is not a device cannot be switched,
/// and members left beside it would mean the curve driving the group
/// and each bulb separately — three commands where one will do, and the
/// bulbs and the group taking turns to overwrite each other.
fn apply_groups(index: &GroupIndex, registry: &DeviceRegistry, bus: &EventBus) -> Vec<DeviceId> {
    let mut ask_state = Vec::new();
    for group in &index.groups {
        // The union of what the members can do. A group holding one
        // colour bulb and one white one can be told a colour; the white
        // one ignores it, exactly as it would if you sent it by hand.
        let capabilities = group
            .members
            .iter()
            .filter_map(|m| index.caps.get(&m.ieee_address))
            .fold(LightCapabilities::default(), |acc, c| LightCapabilities {
                color_temp: acc.color_temp || c.color_temp,
                rgb: acc.rgb || c.rgb,
            });

        let mut device = match group.to_device(capabilities) {
            Ok(device) => device,
            Err(e) => {
                // A group named without a room — `table` rather than
                // `living_room/table`. Worth saying: it will never
                // appear, and nothing else would explain why.
                warn!("group {:?} is not a usable name: {e}", group.friendly_name);
                continue;
            }
        };

        let existing = registry.get(&device.id);
        if let Some(known) = &existing {
            device.state = known.state.clone();
        }
        registry.upsert(device.clone());
        if existing.is_none() {
            debug!("discovered group {}", device.id);
            bus.publish(Event::DeviceAdded {
                device: device.clone(),
            });
        }
        if device.state.on.is_none() {
            ask_state.push(device.id);
        }
    }

    // Members go, so the group is the only way to reach them. Idempotent
    // by construction: `remove` answers `None` for one already gone, and
    // a second pass says nothing.
    for ieee in index.members() {
        let Some(id) = index.ids.get(ieee) else {
            continue;
        };
        if registry.remove(id).is_some() {
            debug!("{id} is in a group; hidden behind it");
            bus.publish(Event::DeviceRemoved { id: id.clone() });
        }
    }

    ask_state
}

/// Parse a `bridge/groups` payload and rebuild from it.
pub(crate) fn handle_group_list(
    payload: &[u8],
    registry: &DeviceRegistry,
    bus: &EventBus,
    index: &Mutex<GroupIndex>,
) -> Vec<DeviceId> {
    let groups = match parse_group_list(payload) {
        Ok(g) => g,
        Err(e) => {
            warn!("failed to parse bridge/groups payload: {e}");
            return Vec::new();
        }
    };
    let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
    index.groups = groups;
    apply_groups(&index, registry, bus)
}

/// Ask Z2M what a device is currently doing.
///
/// Z2M only publishes state when something changes, so a Niles that has
/// just started knows nothing about a light until someone physically
/// turns it off and on — and until then the curve skips it, because it
/// never drives a light it can't confirm is on. Reading `state` makes
/// Z2M publish that device's full cached state, which is everything we
/// need.
///
/// A failure here is not worth interrupting the message loop for: the
/// next state message fixes it, which is exactly where we were before.
async fn request_state(publisher: &crate::client::MqttPublisher, prefix: &str, id: &DeviceId) {
    let topic = format!("{prefix}/{}/{}/get", id.room().as_str(), id.name().as_str());
    if let Err(e) = publisher.publish(&topic, br#"{"state":""}"#.to_vec()).await {
        debug!("could not ask {id} for its state: {e}");
    }
}

/// Route an incoming message to the right handler. Extracted as a
/// free function so tests can exercise routing without constructing a
/// real `MqttClient`.
pub(crate) fn dispatch(
    msg: &Message,
    prefix: &str,
    registry: &DeviceRegistry,
    bus: &EventBus,
    index: &Mutex<GroupIndex>,
) -> Vec<DeviceId> {
    let Some(rest) = msg.topic.strip_prefix(prefix) else {
        return Vec::new();
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return Vec::new();
    };

    if rest == "bridge/devices" {
        return handle_device_list(&msg.payload, registry, bus, index);
    } else if rest == "bridge/groups" {
        return handle_group_list(&msg.payload, registry, bus, index);
    } else if let Some((room, device)) = rest
        .strip_suffix("/availability")
        .and_then(split_room_device)
    {
        if room != "bridge" {
            handle_availability(room, device, &msg.payload, registry, bus);
        }
    } else if let Some((room, device)) = rest.strip_suffix("/action").and_then(split_room_device) {
        // 3-segment `<room>/<device>/action` from a button device.
        // A 2-segment `<room>/action` (state for a flat-named device
        // literally called "action") doesn't parse here and falls
        // through to the state branch below — silently dropping it
        // would have been a regression.
        if room == "bridge" {
            return Vec::new();
        }
        handle_device_action(room, device, &msg.payload, bus);
    } else if let Some((room, device)) = split_room_device(rest) {
        // Skip Z2M's internal `bridge/*` topics other than `bridge/devices`.
        if room == "bridge" {
            return Vec::new();
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
            return Vec::new();
        }
        handle_device_state(room, device, &msg.payload, registry, bus);
    }
    Vec::new()
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
/// Returns the devices whose state nobody has told us yet, so the
/// caller can ask Z2M for it.
pub(crate) fn handle_device_list(
    payload: &[u8],
    registry: &DeviceRegistry,
    bus: &EventBus,
    index: &Mutex<GroupIndex>,
) -> Vec<DeviceId> {
    let devices = match parse_device_list(payload) {
        Ok(d) => d,
        Err(e) => {
            warn!("failed to parse bridge/devices payload: {e}");
            return Vec::new();
        }
    };

    let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
    // What each ieee address is called and can do, which is the half of
    // a group that only this list knows. Recorded for every device,
    // including the ones about to be hidden — a group's capabilities are
    // its members', so they have to survive the hiding.
    index.ids.clear();
    index.caps.clear();
    for z2m in &devices {
        if let Ok(device) = z2m.to_device() {
            index.ids.insert(z2m.ieee_address.clone(), device.id);
            index
                .caps
                .insert(z2m.ieee_address.clone(), z2m.capabilities());
        }
    }
    let hidden = index.members();

    // Build the new set of IDs we want present.
    let mut new_ids: HashSet<DeviceId> = HashSet::new();
    let mut ask_state: Vec<DeviceId> = Vec::new();
    // The groups belong here too. They arrive on their own topic and so
    // are absent from this list — without them the sweep below would
    // remove every group on each reconnect and `apply_groups` would put
    // them straight back, which is a remove and an add on the bus every
    // time Z2M so much as reconnects.
    new_ids.extend(index.group_ids());
    for z2m in &devices {
        if !z2m.is_user_device() {
            continue;
        }
        // A device in a group is reached through the group. Skipped
        // rather than added and then removed, so a house that has had
        // groups since startup never sees the pair of events at all.
        if hidden.contains(z2m.ieee_address.as_str()) {
            continue;
        }
        match z2m.to_device() {
            Ok(mut device) => {
                new_ids.insert(device.id.clone());
                // `to_device` carries no state — it describes what the
                // device *is*, not what it is doing. Z2M republishes
                // this list on every reconnect and every pairing
                // change, so overwriting the entry wholesale forgot
                // what every light was doing, and the curve then
                // skipped them all until each was physically toggled.
                let existing = registry.get(&device.id);
                if let Some(known) = &existing {
                    device.state = known.state.clone();
                }
                registry.upsert(device.clone());
                if existing.is_none() {
                    debug!("discovered {}", device.id);
                    bus.publish(Event::DeviceAdded {
                        device: device.clone(),
                    });
                }
                // Nothing has told us what this one is doing yet. Ask,
                // rather than wait for someone to walk over and flip it.
                if device.state.on.is_none() && device.is_switchable() {
                    ask_state.push(device.id);
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

    // The device list is half the answer; put the groups back on top of
    // it. Cheap when there are none.
    ask_state.extend(apply_groups(&index, registry, bus));
    ask_state
}

/// Maximum action-payload size we'll accept. Z2M action strings are
/// short (`up_hold_release` is the longest at 16 bytes); anything
/// orders of magnitude larger is almost certainly a misconfigured
/// retain or a wrong-topic publish. We log and drop.
const MAX_ACTION_PAYLOAD: usize = 256;

/// Parse a per-device action payload (plain UTF-8 string) and emit a
/// `DeviceAction` event. Drops non-UTF-8 / oversize payloads with a warn.
/// Record whether Z2M can reach a device.
///
/// Published retained, so it arrives again on every reconnect — hence
/// the event only when something actually changed.
pub(crate) fn handle_availability(
    room: &str,
    device: &str,
    payload: &[u8],
    registry: &DeviceRegistry,
    bus: &EventBus,
) {
    let Some(available) = crate::z2m::parse_availability(payload) else {
        debug!("unreadable availability for {room}/{device}; leaving it as it was");
        return;
    };
    let Ok(id) = DeviceId::parse(&format!("z2m:{room}/{device}")) else {
        return;
    };
    if !registry.set_available(&id, available) {
        return;
    }
    debug!(
        "{id} is {}",
        if available {
            "reachable"
        } else {
            "not answering"
        }
    );
    // The same event a state change publishes: what changed about the
    // device is that you can no longer do anything to it, which the
    // dashboard has to hear about the same way.
    bus.publish(niles_core::Event::DeviceStateChanged {
        id: id.clone(),
        state: registry.get(&id).map(|d| d.state).unwrap_or_default(),
    });
}

pub(crate) fn handle_device_action(room: &str, device: &str, payload: &[u8], bus: &EventBus) {
    if payload.len() >= MAX_ACTION_PAYLOAD {
        let len = payload.len();
        warn!(
            "action payload for {room}/{device} is {len} bytes (>= {MAX_ACTION_PAYLOAD}); dropping"
        );
        return;
    }
    let action = match std::str::from_utf8(payload) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            warn!("action payload for {room}/{device} is not UTF-8: {e}");
            return;
        }
    };
    if action.is_empty() {
        warn!("action payload for {room}/{device} is empty after trimming; dropping");
        return;
    }
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
        // Dimmer-style devices can republish `{"action":..,"linkquality":..}`
        // on every press. Those fields aren't tracked state, so skip
        // the merge + event entirely.
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
        handle_device_list(payload, &registry, &bus, &Mutex::default());

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
        handle_device_list(first, &registry, &bus, &Mutex::default());
        drain(&mut rx); // discard initial add events

        let second = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(second, &registry, &bus, &Mutex::default());

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
        handle_device_list(payload, &registry, &bus, &Mutex::default());
        drain(&mut rx); // first add
        handle_device_list(payload, &registry, &bus, &Mutex::default());
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
        handle_device_list(payload, &registry, &bus, &Mutex::default());
        assert!(registry.is_empty());
    }

    #[test]
    fn malformed_device_list_payload_is_logged_not_panicked() {
        let (registry, bus, _rx) = fixtures();
        handle_device_list(b"not json", &registry, &bus, &Mutex::default());
        assert!(registry.is_empty());
    }

    /// Two bulbs, one of which can do colour, and a group holding both.
    const BULBS: &[u8] = br#"[
        {"ieee_address":"0xaa","friendly_name":"living_room/bulb_1","type":"Router",
         "definition":{"exposes":[{"type":"light","features":[{"property":"color_temp"}]}]}},
        {"ieee_address":"0xbb","friendly_name":"living_room/bulb_2","type":"Router",
         "definition":{"exposes":[{"type":"light","features":[{"property":"color_xy"}]}]}},
        {"ieee_address":"0xcc","friendly_name":"office/lamp","type":"Router",
         "definition":{"exposes":[{"type":"light"}]}}
    ]"#;

    const GROUP: &[u8] = br#"[
        {"id":1,"friendly_name":"living_room/table","members":[
            {"ieee_address":"0xaa","endpoint":11},
            {"ieee_address":"0xbb","endpoint":11}
        ]}
    ]"#;

    fn ids(registry: &DeviceRegistry) -> Vec<String> {
        let mut out: Vec<String> = registry
            .list_all()
            .into_iter()
            .map(|d| d.id.to_string())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_group_becomes_a_light_and_its_members_stand_down() {
        let (registry, bus, _rx) = fixtures();
        let index = Mutex::default();
        handle_device_list(BULBS, &registry, &bus, &index);
        handle_group_list(GROUP, &registry, &bus, &index);

        assert_eq!(
            ids(&registry),
            vec!["z2m:living_room/table", "z2m:office/lamp"],
            "the group replaces its members rather than joining them"
        );
    }

    #[test]
    fn the_group_can_be_told_whatever_any_member_can_be_told() {
        // One bulb takes a colour temperature and the other a colour.
        // The union is what the group offers: sending a colour to the
        // white one is Z2M's business, and it is what happens if you
        // send it by hand.
        let (registry, bus, _rx) = fixtures();
        let index = Mutex::default();
        handle_device_list(BULBS, &registry, &bus, &index);
        handle_group_list(GROUP, &registry, &bus, &index);

        let group = registry
            .get(&DeviceId::parse("z2m:living_room/table").unwrap())
            .expect("the group is a device");
        assert!(group.capabilities.color_temp);
        assert!(group.capabilities.rgb);
    }

    #[test]
    fn groups_arriving_first_works_the_same() {
        // The two lists are published independently and either can be
        // first, which on a reconnect is a coin toss.
        let (registry, bus, _rx) = fixtures();
        let index = Mutex::default();
        handle_group_list(GROUP, &registry, &bus, &index);
        handle_device_list(BULBS, &registry, &bus, &index);

        assert_eq!(
            ids(&registry),
            vec!["z2m:living_room/table", "z2m:office/lamp"]
        );

        // And the capabilities catch up. Arriving first, the group had
        // no device list to read its members from and could only be
        // told on and off; the list landing fixes it, because every
        // device list rebuilds the groups on top of itself.
        let group = registry
            .get(&DeviceId::parse("z2m:living_room/table").unwrap())
            .expect("the group is a device");
        assert!(group.capabilities.color_temp);
        assert!(group.capabilities.rgb);
    }

    #[test]
    fn a_device_list_refresh_leaves_the_group_alone() {
        // Z2M republishes bridge/devices on every reconnect, and groups
        // are not in it. Without holding on to them the sweep would
        // remove every group and put it straight back — a removal and
        // an addition on the bus each time.
        let (registry, bus, mut rx) = fixtures();
        let index = Mutex::default();
        handle_device_list(BULBS, &registry, &bus, &index);
        handle_group_list(GROUP, &registry, &bus, &index);
        let _ = drain(&mut rx);

        handle_device_list(BULBS, &registry, &bus, &index);
        let events = drain(&mut rx);
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::DeviceRemoved { id } if id.name().as_str() == "table"
            )),
            "the group should not have been removed: {events:?}"
        );
        assert!(
            registry
                .get(&DeviceId::parse("z2m:living_room/table").unwrap())
                .is_some()
        );
    }

    #[test]
    fn a_group_without_a_room_is_reported_rather_than_dropped_silently() {
        // `table` has no room segment, so it can never be a device id.
        // It will simply never appear, and nothing else would say why.
        let (registry, bus, _rx) = fixtures();
        let index = Mutex::default();
        handle_device_list(BULBS, &registry, &bus, &index);
        let flat = br#"[{"id":2,"friendly_name":"table","members":[]}]"#;
        handle_group_list(flat, &registry, &bus, &index);

        assert_eq!(ids(&registry).len(), 3, "nothing was added or hidden");
    }

    #[test]
    fn a_device_list_refresh_keeps_what_we_know_about_a_light() {
        // Z2M republishes bridge/devices on every reconnect and every
        // pairing change. Overwriting the entry wholesale forgot what
        // each light was doing, and the curve then skipped all of them
        // until someone physically toggled each one.
        let (registry, bus, _rx) = fixtures();
        let id = DeviceId::parse("z2m:office/lightstrip").unwrap();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"office/lightstrip","type":"Router",
             "definition":{"exposes":[{"type":"light"}]}}
        ]"#;
        handle_device_list(payload, &registry, &bus, &Mutex::default());

        handle_device_state(
            "office",
            "lightstrip",
            br#"{"state":"ON","brightness":200,"color_temp":370}"#,
            &registry,
            &bus,
        );
        assert_eq!(registry.get(&id).unwrap().state.on, Some(true));

        handle_device_list(payload, &registry, &bus, &Mutex::default());
        let after = registry.get(&id).unwrap();
        assert_eq!(after.state.on, Some(true), "state survives the refresh");
        assert!(after.state.brightness.is_some());
        assert!(after.state.color_temp_kelvin.is_some());
    }

    #[test]
    fn a_light_we_know_nothing_about_is_asked() {
        let (registry, bus, _rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"office/lightstrip","type":"Router",
             "definition":{"exposes":[{"type":"light"}]}}
        ]"#;
        let ask = handle_device_list(payload, &registry, &bus, &Mutex::default());
        assert_eq!(
            ask,
            vec![DeviceId::parse("z2m:office/lightstrip").unwrap()],
            "nothing has said whether this light is on, so ask"
        );
    }

    #[test]
    fn a_light_we_already_know_about_is_not_asked_again() {
        let (registry, bus, _rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"office/lightstrip","type":"Router",
             "definition":{"exposes":[{"type":"light"}]}}
        ]"#;
        handle_device_list(payload, &registry, &bus, &Mutex::default());
        handle_device_state(
            "office",
            "lightstrip",
            br#"{"state":"ON"}"#,
            &registry,
            &bus,
        );

        assert!(
            handle_device_list(payload, &registry, &bus, &Mutex::default()).is_empty(),
            "we already know; asking again is pointless traffic"
        );
    }

    #[test]
    fn a_sensor_is_never_asked() {
        // Reading from a battery device wakes it for nothing.
        let (registry, bus, _rx) = fixtures();
        let payload = br#"[
            {"ieee_address":"0x1","friendly_name":"hallway/motion","type":"EndDevice",
             "definition":{"exposes":[{"type":"binary","property":"occupancy"}]}}
        ]"#;
        assert!(handle_device_list(payload, &registry, &bus, &Mutex::default()).is_empty());
    }

    // ---- handle_device_state -------------------------------------

    #[test]
    fn state_message_updates_registry_and_emits_event() {
        let (registry, bus, mut rx) = fixtures();
        let device_list = br#"[
            {"ieee_address":"0x1","friendly_name":"kitchen/ceiling_light","type":"Router"}
        ]"#;
        handle_device_list(device_list, &registry, &bus, &Mutex::default());
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
        handle_device_list(device_list, &registry, &bus, &Mutex::default());
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
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
        assert_eq!(registry.list_all().len(), 1);
    }

    #[test]
    fn dispatch_routes_state_topics() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/office/desk_lamp".into(),
            payload: br#"{"state":"ON"}"#.to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
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
            dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
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
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
        assert!(registry.is_empty());
    }

    #[test]
    fn dispatch_ignores_z2m_internal_topics() {
        let (registry, bus, _rx) = fixtures();
        let msg = Message {
            topic: "zigbee2mqtt/bridge/logging".into(),
            payload: br#"{"level":"info"}"#.to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
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

    #[test]
    fn action_with_trailing_newline_is_trimmed() {
        let (_registry, bus, mut rx) = fixtures();
        handle_device_action("office", "switch", b"on_press\n", &bus);
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
    fn action_with_whitespace_only_payload_is_dropped() {
        let (_registry, bus, mut rx) = fixtures();
        handle_device_action("office", "switch", b"   \n\t", &bus);
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
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
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
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn dispatch_two_segment_action_falls_through_to_state() {
        // `zigbee2mqtt/office/action` is a 2-segment state topic for a
        // flat-named device literally called "action" — not our
        // 3-segment action topic. Pre-PR this was handled as state;
        // the action branch must fall through so it stays that way.
        let (registry, bus, mut rx) = fixtures();
        let device_list = br#"[
            {"ieee_address":"0x1","friendly_name":"office/action","type":"EndDevice"}
        ]"#;
        handle_device_list(device_list, &registry, &bus, &Mutex::default());
        drain(&mut rx);

        let msg = Message {
            topic: "zigbee2mqtt/office/action".into(),
            payload: br#"{"state":"ON"}"#.to_vec(),
        };
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());

        let id = DeviceId::parse("z2m:office/action").unwrap();
        assert_eq!(registry.get(&id).unwrap().state.on, Some(true));
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::DeviceStateChanged { .. }));
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
        dispatch(&msg, "zigbee2mqtt", &registry, &bus, &Mutex::default());
        assert!(drain(&mut rx).is_empty());
    }

    // ---- JSON state filter --------------------------------------

    #[test]
    fn state_with_only_action_field_does_not_emit() {
        // Z2M can publish `{"action":..,"linkquality":..}` alone when
        // the dimmer's battery hasn't changed. No tracked field is set,
        // so the dispatch path must skip it.
        let (registry, bus, mut rx) = fixtures();
        handle_device_state(
            "office",
            "switch",
            br#"{"action":"on_press","linkquality":168}"#,
            &registry,
            &bus,
        );
        assert!(
            drain(&mut rx).is_empty(),
            "action-only payload must not emit DeviceStateChanged"
        );
    }

    #[test]
    fn battery_only_state_payload_still_emits() {
        // Regression guard: battery is surfaced via the HTTP API and
        // the `get_device_state` tool, so a `{"battery":..}` update
        // must merge into the registry and fire DeviceStateChanged.
        let (registry, bus, mut rx) = fixtures();
        let device_list = br#"[
            {"ieee_address":"0x1","friendly_name":"office/sensor","type":"EndDevice"}
        ]"#;
        handle_device_list(device_list, &registry, &bus, &Mutex::default());
        drain(&mut rx);

        handle_device_state("office", "sensor", br#"{"battery":42}"#, &registry, &bus);

        let id = DeviceId::parse("z2m:office/sensor").unwrap();
        assert_eq!(registry.get(&id).unwrap().state.battery_percent, Some(42));
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::DeviceStateChanged { .. }));
    }
}

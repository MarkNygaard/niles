//! Which devices a command reaches, and publishing it to them.

use crate::dispatch::DispatchCtx;
use crate::wiring::automations::command_directly;
use niles_core::{Device, DeviceId, DeviceRegistry, DeviceState, RoomName};
use niles_mqtt::is_actionable;
use std::collections::HashSet;
use std::net::SocketAddr;

/// Outcome of resolving a room name to a device list.
pub(crate) enum RoomResolve {
    Found(RoomName, Vec<Device>),
    BadName,
    NoDevices,
    WarmingUp,
}

/// Resolve a transcript-derived room reference into the canonical
/// `RoomName` + the list of devices in that room that pass
/// `capability_filter`.
///
/// Centralizing this avoids the previous duplicated lookup in the
/// `LightDim` arm (one walk to flag, another inside `dispatch_room`
/// to publish).
pub(crate) fn resolve_room_targets<F>(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    room: &str,
    capability_filter: F,
) -> RoomResolve
where
    F: Fn(&niles_core::Device) -> bool,
{
    let canonical = match intent_room_to_canonical(room) {
        Ok(r) => r,
        Err(reason) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            return RoomResolve::BadName;
        }
    };

    let targets: Vec<_> = ctx
        .registry
        .list_room(&canonical)
        .into_iter()
        .filter(&capability_filter)
        .collect();

    if targets.is_empty() {
        // Distinguish startup race from a genuinely empty room.
        // The registry is populated asynchronously from Z2M's
        // retained `bridge/devices`; on a cold start it may not
        // have arrived yet by the time the first utterance lands.
        if ctx.registry.is_empty() {
            println!(
                "[{peer}] registry is still warming up (no devices yet) — try again in a moment"
            );
            RoomResolve::WarmingUp
        } else {
            println!(
                "[{peer}] no devices in room '{canonical}' support this action — nothing to dispatch"
            );
            RoomResolve::NoDevices
        }
    } else {
        RoomResolve::Found(canonical, targets)
    }
}

/// Publish the requested target state to each device in `targets`.
/// Pure dispatch — room resolution + capability filtering happened
/// upstream in [`resolve_room_targets`].
pub(crate) async fn dispatch_to_targets(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    targets: &[Device],
    desired: &DeviceState,
) {
    debug_assert!(
        is_actionable(desired),
        "dispatch_to_targets called with a non-actionable target state"
    );

    for device in targets {
        if let Some(commander) = ctx.router.commander_for(&device.id, desired) {
            let what = format!("[{peer}]");
            command_directly(commander, &device.id, desired, ctx.dry_run, &what).await;
            continue;
        }
        let Some((topic, payload)) = ctx.router.format(&device.id, desired) else {
            continue;
        };
        if ctx.dry_run {
            tracing::info!("[{peer}] [dry-run] {topic}  {payload}");
            continue;
        }
        match ctx.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => tracing::info!("[{peer}] published {topic}  {payload}"),
            Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
        }
    }
}

/// Publish a state update to a single device.
///
/// The caller is responsible for `tracker.flag()` timing: flag
/// *before* publish for dim operations so the curve driver can't
/// race, and *after* publish for on/off (same pattern as
/// `dispatch_to_targets`).
pub(crate) async fn publish_single(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    device: &Device,
    desired: &DeviceState,
) {
    if let Some(commander) = ctx.router.commander_for(&device.id, desired) {
        let what = format!("[{peer}]");
        command_directly(commander, &device.id, desired, ctx.dry_run, &what).await;
        return;
    }
    let Some((topic, payload)) = ctx.router.format(&device.id, desired) else {
        return;
    };
    if ctx.dry_run {
        tracing::info!("[{peer}] [dry-run] {topic}  {payload}");
    } else {
        match ctx.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => tracing::info!("[{peer}] published {topic}  {payload}"),
            Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
        }
    }
}

/// Convert a transcript-style room reference ("living room") into a
/// registry [`RoomName`] ("living_room"). This is the single
/// normalization point between intent output and registry lookup:
/// trims, lowercases, swaps whitespace for underscores, then
/// validates via `RoomName::parse`.
pub(crate) fn intent_room_to_canonical(s: &str) -> std::result::Result<RoomName, String> {
    let normalized = s.trim().to_ascii_lowercase().replace([' ', '\t'], "_");
    RoomName::parse(&normalized).map_err(|e| format!("{e}"))
}

/// Room name reserved for whole-home switches: a switch in room `all`
/// (e.g. `all/bedroom_switch`) controls every light in the house, not
/// just one room's — handy for a bedside "all off" or a door "leaving"
/// button.
pub(crate) const ALL_ROOMS: &str = "all";

/// Devices a switch press should act on: the lights — including a lamp
/// on a plug, but not whatever else is on one. A switch in the reserved
/// `all` room targets the whole home; any other room targets just that
/// room. The switch itself is always excluded.
/// Whether a device's actions are presses that switch its room. Any
/// device may report actions, and a plugin's printer saying `off` is
/// not somebody at the door pressing off. One Niles does not know is
/// taken at its word, as before actions came from anything but switches.
pub(crate) fn is_wall_switch(registry: &DeviceRegistry, id: &DeviceId) -> bool {
    registry
        .get(id)
        .is_none_or(|d| d.class == niles_core::DeviceClass::Switch)
}

pub(crate) fn switch_targets(
    registry: &DeviceRegistry,
    switch_id: &DeviceId,
    lamp_plugs: &HashSet<DeviceId>,
) -> Vec<Device> {
    let room = switch_id.room();
    let candidates = if room.as_str() == ALL_ROOMS {
        registry.list_all()
    } else {
        registry.list_room(room)
    };
    candidates
        .into_iter()
        .filter(|d| d.is_lamp(lamp_plugs) && &d.id != switch_id)
        .collect()
}

#[cfg(test)]
mod switch_target_tests {
    use super::*;
    use niles_core::{DeviceClass, DeviceState};

    fn dev(id: &str, class: DeviceClass) -> Device {
        Device::new(DeviceId::parse(id).unwrap(), DeviceState::default(), class)
    }
    fn light(id: &str) -> Device {
        dev(id, DeviceClass::Light)
    }

    #[test]
    fn only_a_switch_switches_its_room() {
        let reg = DeviceRegistry::new();
        reg.upsert(dev("z2m:office/switch", DeviceClass::Switch));
        reg.upsert(dev("x_bambulab:office/printer", DeviceClass::Sensor));
        assert!(is_wall_switch(
            &reg,
            &DeviceId::parse("z2m:office/switch").unwrap()
        ));
        assert!(!is_wall_switch(
            &reg,
            &DeviceId::parse("x_bambulab:office/printer").unwrap()
        ));
    }

    #[test]
    fn a_light_kept_out_of_the_room_is_not_switched_with_it() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:office/lamp"));
        let mut chamber = light("x_bambulab:office/chamber_light");
        chamber.room_light = false;
        reg.upsert(chamber);
        let sw = DeviceId::parse("z2m:office/switch").unwrap();
        let targets = switch_targets(&reg, &sw, &HashSet::new());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id.to_string(), "z2m:office/lamp");
    }

    #[test]
    fn room_switch_targets_only_its_room() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:bedroom/lamp"));
        reg.upsert(light("z2m:kitchen/ceiling"));
        let sw = DeviceId::parse("z2m:bedroom/main_switch").unwrap();
        let targets = switch_targets(&reg, &sw, &HashSet::new());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id.to_string(), "z2m:bedroom/lamp");
    }

    fn lamp(id: &str) -> HashSet<DeviceId> {
        HashSet::from([DeviceId::parse(id).unwrap()])
    }

    #[test]
    fn targets_include_lamp_plugs_not_sensors() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:living_room/ceiling"));
        reg.upsert(dev("z2m:living_room/corner_lamp", DeviceClass::Outlet)); // Hue plug
        reg.upsert(dev("z2m:living_room/thermometer", DeviceClass::Sensor));
        let sw = DeviceId::parse("z2m:living_room/main_switch").unwrap();
        let lamps = lamp("z2m:living_room/corner_lamp");
        let mut names: Vec<String> = switch_targets(&reg, &sw, &lamps)
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        names.sort();
        // The lamp's plug is included; the sensor is not.
        assert_eq!(
            names,
            ["z2m:living_room/ceiling", "z2m:living_room/corner_lamp"]
        );
    }

    #[test]
    fn a_plug_nobody_called_a_lamp_is_left_alone() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:kitchen/ceiling"));
        reg.upsert(dev("z2m:kitchen/coffee_machine", DeviceClass::Outlet));
        let sw = DeviceId::parse("z2m:kitchen/switch").unwrap();
        let targets = switch_targets(&reg, &sw, &HashSet::new());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id.to_string(), "z2m:kitchen/ceiling");
    }

    #[test]
    fn all_room_switch_targets_every_lamp() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:bedroom/lamp"));
        reg.upsert(light("wled:living_room/ceiling"));
        reg.upsert(dev("z2m:living_room/corner_lamp", DeviceClass::Outlet));
        let sw = DeviceId::parse("z2m:all/bedroom_switch").unwrap();
        let lamps = lamp("z2m:living_room/corner_lamp");
        let mut names: Vec<String> = switch_targets(&reg, &sw, &lamps)
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "wled:living_room/ceiling",
                "z2m:bedroom/lamp",
                "z2m:living_room/corner_lamp"
            ]
        );
    }

    #[test]
    fn switch_never_targets_itself() {
        let reg = DeviceRegistry::new();
        reg.upsert(light("z2m:all/lamp"));
        let sw = DeviceId::parse("z2m:all/lamp").unwrap(); // same id as a light
        assert!(
            switch_targets(&reg, &sw, &HashSet::new())
                .iter()
                .all(|d| d.id != sw)
        );
    }
}

#[cfg(test)]
mod intent_room_tests {
    use super::*;

    #[test]
    fn single_word_room_passes_through() {
        let r = intent_room_to_canonical("kitchen").unwrap();
        assert_eq!(r.as_str(), "kitchen");
    }

    #[test]
    fn multi_word_room_replaces_spaces_with_underscores() {
        let r = intent_room_to_canonical("living room").unwrap();
        assert_eq!(r.as_str(), "living_room");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let r = intent_room_to_canonical("  bedroom  ").unwrap();
        assert_eq!(r.as_str(), "bedroom");
    }

    #[test]
    fn mixed_case_lowercases() {
        // Normalization is this function's contract — callers don't
        // have to pre-lowercase.
        let r = intent_room_to_canonical("Kitchen").unwrap();
        assert_eq!(r.as_str(), "kitchen");
    }

    #[test]
    fn rejects_invalid_chars() {
        // RoomName::parse rejects anything outside [a-z0-9_].
        assert!(intent_room_to_canonical("kitchen!").is_err());
        assert!(intent_room_to_canonical("kitchen-light").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(intent_room_to_canonical("").is_err());
        assert!(intent_room_to_canonical("   ").is_err());
    }
}

//! Z2M command formatting — produces the topic + JSON payload to
//! publish for a Z2M "set" command.
//!
//! Pure conversion. Callers publish the result via their own
//! `MqttClient`. Keeping this side-effect-free means it's trivially
//! unit-testable without a broker, and the publish layer (with its
//! reconnect + backpressure concerns) stays in `client.rs`.
//!
//! ## Sensor fields are ignored
//!
//! `DeviceState` has fields like `temperature_celsius` that come
//! *from* devices but aren't settable *on* them. Those fields are
//! silently dropped from set-command payloads — real Z2M devices
//! would reject them anyway.

use niles_core::{DeviceId, DeviceState};
use serde::Serialize;

/// Build the topic + JSON payload for a Z2M `set` command.
///
/// `prefix` is the Z2M topic root (typically `"zigbee2mqtt"`).
/// Only the *settable* fields of `target` (`on`, `brightness`,
/// `color_temp_kelvin`) end up in the payload; sensor fields are
/// silently ignored.
///
/// If `target` has no settable fields set, the payload is `{}` —
/// the caller should generally check for that and not publish a
/// no-op message.
pub fn format_set_command(prefix: &str, id: &DeviceId, target: &DeviceState) -> (String, String) {
    let topic = format!("{}/{}/{}/set", prefix, id.room(), id.name());
    let payload = Z2mSetPayload::from(target);
    let json = serde_json::to_string(&payload).expect("Z2mSetPayload always serializes");
    (topic, json)
}

/// Returns `true` if a `DeviceState` has at least one field Z2M will
/// honor as a command. Use this to skip publishing no-op messages.
pub fn is_actionable(target: &DeviceState) -> bool {
    target.on.is_some()
        || target.brightness.is_some()
        || target.color_temp_kelvin.is_some()
        || target.rgb.is_some()
}

#[derive(Debug, Serialize)]
struct Z2mSetPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_temp: Option<u16>,
}

impl From<&DeviceState> for Z2mSetPayload {
    fn from(s: &DeviceState) -> Self {
        Self {
            state: s.on.map(|on| if on { "ON" } else { "OFF" }),
            brightness: s.brightness.map(percent_to_z2m_brightness),
            color_temp: s.color_temp_kelvin.and_then(kelvin_to_mireds),
        }
    }
}

/// Niles brightness `0..=100` → Z2M's `0..=254`, rounded to nearest.
/// Clamps inputs > 100 (defensive against future mistakes — Niles's
/// own type is `u8` which allows `>100`).
fn percent_to_z2m_brightness(pct: u8) -> u16 {
    let pct = u32::from(pct.min(100));
    // (pct * 254 + 50) / 100 = round half-up.
    ((pct * 254 + 50) / 100) as u16
}

/// Kelvin → mireds. Returns `None` for `kelvin == 0` (avoids the
/// `1_000_000 / 0` divide).
fn kelvin_to_mireds(kelvin: u16) -> Option<u16> {
    if kelvin == 0 {
        return None;
    }
    let mireds = 1_000_000_u32 / u32::from(kelvin);
    Some(mireds.try_into().unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(room: &str, name: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{room}/{name}")).unwrap()
    }

    // ---- conversions ---------------------------------------------

    #[test]
    fn percent_to_brightness_extremes() {
        assert_eq!(percent_to_z2m_brightness(0), 0);
        assert_eq!(percent_to_z2m_brightness(100), 254);
    }

    #[test]
    fn percent_to_brightness_midpoint() {
        // 50% should map to half of 254, i.e. 127.
        assert_eq!(percent_to_z2m_brightness(50), 127);
    }

    #[test]
    fn percent_to_brightness_clamps_over_100() {
        assert_eq!(percent_to_z2m_brightness(200), 254);
    }

    #[test]
    fn brightness_round_trips_at_endpoints() {
        // Defensive against the reverse conversion in z2m.rs: a
        // Niles brightness of 0 or 100, run forward through this
        // module and back through z2m.rs's parser, should land
        // exactly. (Mid-range values can lose 1 unit out of 254
        // due to int rounding; that's documented as acceptable.)
        for pct in [0u8, 100] {
            let z2m_value = percent_to_z2m_brightness(pct);
            // Reverse formula: see `z2m::z2m_brightness_to_percent`.
            let back = ((u32::from(z2m_value) * 100 + 127) / 254) as u8;
            assert_eq!(back, pct, "round-trip failed for {pct}%");
        }
    }

    #[test]
    fn kelvin_to_mireds_known_values() {
        // 4000K → 250 mireds, 2700K → 370, 6500K → 153.
        assert_eq!(kelvin_to_mireds(4000), Some(250));
        assert_eq!(kelvin_to_mireds(2700), Some(370));
        assert_eq!(kelvin_to_mireds(6500), Some(153));
    }

    #[test]
    fn kelvin_zero_is_none() {
        assert_eq!(kelvin_to_mireds(0), None);
    }

    // ---- format_set_command --------------------------------------

    #[test]
    fn topic_includes_prefix_room_device_and_set_suffix() {
        let (topic, _) = format_set_command(
            "zigbee2mqtt",
            &id("kitchen", "ceiling_light"),
            &DeviceState {
                on: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(topic, "zigbee2mqtt/kitchen/ceiling_light/set");
    }

    #[test]
    fn payload_contains_only_settable_fields() {
        let (_, payload) = format_set_command(
            "zigbee2mqtt",
            &id("kitchen", "ceiling_light"),
            &DeviceState {
                on: Some(true),
                brightness: Some(80),
                color_temp_kelvin: Some(2700),
                rgb: None,
                // sensor fields — must be skipped:
                temperature_celsius: Some(21.5),
                humidity_percent: Some(50.0),
                battery_percent: Some(99),
            },
        );
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["state"], "ON");
        assert_eq!(v["brightness"], 203); // (80*254 + 50) / 100 = 20370/100 = 203
        assert_eq!(v["color_temp"], 370); // 1_000_000 / 2700 = 370
        // No sensor fields:
        assert!(v.get("temperature").is_none());
        assert!(v.get("temperature_celsius").is_none());
        assert!(v.get("humidity").is_none());
        assert!(v.get("battery").is_none());
    }

    #[test]
    fn off_serializes_as_off_uppercase() {
        let (_, payload) = format_set_command(
            "zigbee2mqtt",
            &id("kitchen", "ceiling_light"),
            &DeviceState {
                on: Some(false),
                ..Default::default()
            },
        );
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["state"], "OFF");
        assert_eq!(v.as_object().unwrap().len(), 1, "no other fields");
    }

    #[test]
    fn empty_state_yields_empty_object() {
        let (_, payload) = format_set_command(
            "zigbee2mqtt",
            &id("kitchen", "ceiling_light"),
            &DeviceState::default(),
        );
        assert_eq!(payload, "{}");
    }

    #[test]
    fn is_actionable_reports_settable_fields() {
        assert!(!is_actionable(&DeviceState::default()));
        assert!(is_actionable(&DeviceState {
            on: Some(false),
            ..Default::default()
        }));
        assert!(is_actionable(&DeviceState {
            brightness: Some(50),
            ..Default::default()
        }));
        assert!(is_actionable(&DeviceState {
            color_temp_kelvin: Some(3000),
            ..Default::default()
        }));
        assert!(is_actionable(&DeviceState {
            rgb: Some([255, 128, 0]),
            ..Default::default()
        }));
        // Sensor-only state is not actionable:
        assert!(!is_actionable(&DeviceState {
            temperature_celsius: Some(20.0),
            humidity_percent: Some(50.0),
            battery_percent: Some(88),
            ..Default::default()
        }));
    }

    #[test]
    fn custom_prefix_is_respected() {
        let (topic, _) = format_set_command(
            "z2m-custom",
            &id("office", "desk_lamp"),
            &DeviceState {
                on: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(topic, "z2m-custom/office/desk_lamp/set");
    }
}

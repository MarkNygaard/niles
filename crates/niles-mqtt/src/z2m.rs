//! Zigbee2MQTT message types and conversions to `niles-core` types.
//!
//! Z2M publishes two kinds of payload we care about for v0.1:
//!
//! 1. `<prefix>/bridge/devices` — a JSON array of all paired devices
//!    with their `friendly_name` (this is where our `<room>/<device>`
//!    convention lives) and `type` (Coordinator / Router / EndDevice).
//! 2. `<prefix>/<friendly_name>` — per-device state JSON with the
//!    capability values (`state`, `brightness`, `color_temp`, etc.).
//!
//! `serde(deny_unknown_fields)` is deliberately **not** used here —
//! Z2M's schema evolves and we want forward compatibility.

use niles_core::{Device, DeviceClass, DeviceId, DeviceState};
use serde::Deserialize;

/// A single entry from `<prefix>/bridge/devices`.
///
/// Only the fields we care about are typed; Z2M sends many more
/// (endpoints, network_address, ...) which we ignore for v0.1.
/// `definition.exposes` is now parsed to derive [`DeviceClass`].
#[derive(Debug, Clone, Deserialize)]
pub struct Z2mDevice {
    /// e.g. `"0x00124b001f44b3e6"`. We don't currently use this — the
    /// canonical Niles identifier is the `<room>/<device>` form from
    /// `friendly_name` — but it's the stable hardware ID in case we
    /// need it later for handling renames.
    pub ieee_address: String,
    /// `<room>/<device>` per the Niles naming convention.
    pub friendly_name: String,
    /// `"Coordinator"` / `"Router"` / `"EndDevice"`.
    #[serde(rename = "type")]
    pub device_type: String,
    /// Z2M device definition (model, exposes, etc.).
    pub definition: Option<Z2mDefinition>,
}

/// Z2M device definition metadata.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Z2mDefinition {
    /// List of capability exposes for this device.
    pub exposes: Vec<Z2mExpose>,
}

/// A single expose entry within `definition.exposes`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Z2mExpose {
    /// Expose type, e.g. `"light"`, `"switch"`, `"numeric"`, etc.
    #[serde(rename = "type")]
    pub expose_type: Option<String>,
    /// Property name, e.g. `"action"`, `"temperature"`, etc.
    pub property: Option<String>,
    /// Nested exposes (e.g. a light expose contains features).
    pub features: Vec<Z2mExpose>,
}

impl Z2mDevice {
    /// Coordinators don't represent user-visible devices and should
    /// be skipped when populating the registry.
    pub fn is_user_device(&self) -> bool {
        self.device_type != "Coordinator"
    }

    /// Classify this Z2M device into a [`DeviceClass`] based on
    /// `definition.exposes`.
    ///
    /// Rules (in priority order):
    /// - exposes contains `{"type": "light"}` → `Light`
    /// - exposes contains `{"property": "action"}` (no light) → `Switch`
    /// - non-empty exposes, neither light nor action → `Sensor`
    /// - missing/empty definition → `Unknown`
    pub fn classify(&self) -> DeviceClass {
        let exposes = match self.definition.as_ref() {
            Some(def) if !def.exposes.is_empty() => &def.exposes,
            _ => return DeviceClass::Unknown,
        };

        let mut has_action = false;

        for expose in exposes {
            if Self::expose_is_light(expose) {
                return DeviceClass::Light;
            }
            if Self::expose_is_action(expose) {
                has_action = true;
            }
        }

        if has_action {
            DeviceClass::Switch
        } else {
            DeviceClass::Sensor
        }
    }

    fn expose_is_light(expose: &Z2mExpose) -> bool {
        if expose.expose_type.as_deref() == Some("light") {
            return true;
        }
        expose.features.iter().any(Self::expose_is_light)
    }

    fn expose_is_action(expose: &Z2mExpose) -> bool {
        if expose.property.as_deref() == Some("action") {
            return true;
        }
        expose.features.iter().any(Self::expose_is_action)
    }

    /// Convert into a `niles_core::Device` with a default (empty) state
    /// and a [`DeviceClass`] derived via [`Self::classify`].
    ///
    /// Returns an error if the `friendly_name` doesn't parse as a
    /// valid `<room>/<device>` identifier.
    pub fn to_device(&self) -> crate::Result<Device> {
        let id = DeviceId::parse(&format!("z2m:{}", self.friendly_name))?;
        Ok(Device::new(id, DeviceState::default(), self.classify()))
    }
}

/// Per-device state payload from `<prefix>/<friendly_name>`.
///
/// All fields optional — Z2M only sends what changed. `#[serde(default)]`
/// at the struct level fills missing fields with `None`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Z2mState {
    /// `"ON"` / `"OFF"`. Anything else is ignored.
    pub state: Option<String>,
    /// 0–254 (Z2M's range), translated to 0–100 percent.
    pub brightness: Option<u16>,
    /// Color temperature in mireds; converted to Kelvin.
    pub color_temp: Option<u16>,
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    pub battery: Option<f32>,
}

impl Z2mState {
    pub fn to_device_state(&self) -> DeviceState {
        DeviceState {
            on: self.state.as_deref().and_then(parse_on_off),
            brightness: self.brightness.map(z2m_brightness_to_percent),
            color_temp_kelvin: self.color_temp.and_then(mireds_to_kelvin),
            temperature_celsius: self.temperature,
            humidity_percent: self.humidity,
            battery_percent: self.battery.map(|b| b.round().clamp(0.0, 100.0) as u8),
        }
    }

    /// True if any tracked state field is present in the parsed
    /// payload. Used to filter out Z2M's noisy "pure-action" publishes
    /// from button-style devices: the dimmer republishes
    /// `{"action":..,"linkquality":..}` on every press, and without
    /// this filter that would look like an empty state change.
    ///
    /// Battery counts as actionable — `battery_percent` is surfaced via
    /// the HTTP API and the `get_device_state` tool, so a battery-only
    /// payload from a sensor must still propagate. The dimmer's
    /// payloads commonly include `battery`, which means they too will
    /// pass this filter; that's deliberate (we want the dimmer's
    /// battery tracked too).
    pub fn has_actionable_state_field(&self) -> bool {
        self.state.is_some()
            || self.brightness.is_some()
            || self.color_temp.is_some()
            || self.temperature.is_some()
            || self.humidity.is_some()
            || self.battery.is_some()
    }
}

fn parse_on_off(s: &str) -> Option<bool> {
    match s {
        "ON" => Some(true),
        "OFF" => Some(false),
        _ => None,
    }
}

/// Z2M brightness is 0–254. Translate to a percent in 0..=100,
/// rounding to nearest.
fn z2m_brightness_to_percent(z2m: u16) -> u8 {
    let pct = (u32::from(z2m) * 100 + 127) / 254;
    pct.min(100) as u8
}

/// Mireds (Z2M) → Kelvin. `mireds == 0` is meaningless and produces
/// `None` rather than infinity.
fn mireds_to_kelvin(mireds: u16) -> Option<u16> {
    if mireds == 0 {
        return None;
    }
    let kelvin = 1_000_000_u32 / u32::from(mireds);
    Some(kelvin.try_into().unwrap_or(u16::MAX))
}

/// Parse the body of a `<prefix>/bridge/devices` message — a JSON
/// array of `Z2mDevice`.
pub fn parse_device_list(json: &[u8]) -> crate::Result<Vec<Z2mDevice>> {
    Ok(serde_json::from_slice(json)?)
}

/// Parse the body of a `<prefix>/<friendly_name>` state message.
pub fn parse_state(json: &[u8]) -> crate::Result<Z2mState> {
    Ok(serde_json::from_slice(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- unit conversions ----------------------------------------

    #[test]
    fn brightness_extremes() {
        assert_eq!(z2m_brightness_to_percent(0), 0);
        assert_eq!(z2m_brightness_to_percent(254), 100);
    }

    #[test]
    fn brightness_midpoint_rounds_to_50() {
        // 127 / 254 = 0.5 exactly. With round-to-nearest we want 50.
        assert_eq!(z2m_brightness_to_percent(127), 50);
    }

    #[test]
    fn brightness_above_254_clamps() {
        // Z2M shouldn't send this but be defensive.
        assert_eq!(z2m_brightness_to_percent(500), 100);
    }

    #[test]
    fn mireds_to_kelvin_known_values() {
        // 250 mireds = 4000K (standard "neutral white")
        assert_eq!(mireds_to_kelvin(250), Some(4000));
        // 370 mireds ≈ 2700K (warm white)
        assert_eq!(mireds_to_kelvin(370), Some(2702));
        // 154 mireds ≈ 6500K (daylight)
        assert_eq!(mireds_to_kelvin(154), Some(6493));
    }

    #[test]
    fn mireds_zero_is_none() {
        assert_eq!(mireds_to_kelvin(0), None);
    }

    #[test]
    fn parse_on_off_known() {
        assert_eq!(parse_on_off("ON"), Some(true));
        assert_eq!(parse_on_off("OFF"), Some(false));
        assert_eq!(parse_on_off("on"), None); // case sensitive
        assert_eq!(parse_on_off("toggle"), None);
    }

    // ---- bridge/devices parsing ----------------------------------

    #[test]
    fn parses_minimal_device_list() {
        let json = br#"[
            {
                "ieee_address": "0x00124b001cd4bbf0",
                "friendly_name": "Coordinator",
                "type": "Coordinator"
            },
            {
                "ieee_address": "0x00124b001f44b3e6",
                "friendly_name": "kitchen/ceiling_light",
                "type": "Router"
            }
        ]"#;
        let devices = parse_device_list(json).unwrap();
        assert_eq!(devices.len(), 2);
        assert!(!devices[0].is_user_device());
        assert!(devices[1].is_user_device());
    }

    #[test]
    fn tolerates_unknown_fields() {
        // Z2M sends many fields we ignore. The parser must accept them.
        let json = br#"[{
            "ieee_address": "0x123",
            "friendly_name": "office/desk_lamp",
            "type": "EndDevice",
            "definition": { "model": "X", "vendor": "Y" },
            "endpoints": {},
            "supported": true,
            "interview_completed": true,
            "network_address": 1234
        }]"#;
        let devices = parse_device_list(json).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].friendly_name, "office/desk_lamp");
    }

    #[test]
    fn device_to_niles_core() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/ceiling_light".into(),
            device_type: "Router".into(),
            definition: None,
        };
        let device = z2m.to_device().unwrap();
        assert_eq!(device.id.source(), "z2m");
        assert_eq!(device.id.room().as_str(), "kitchen");
        assert_eq!(device.id.name().as_str(), "ceiling_light");
        assert_eq!(device.class, DeviceClass::Unknown);
    }

    #[test]
    fn device_with_malformed_friendly_name_fails() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "Kitchen Light".into(), // uppercase + space
            device_type: "Router".into(),
            definition: None,
        };
        assert!(z2m.to_device().is_err());
    }

    // ---- classification ------------------------------------------

    #[test]
    fn classify_light_from_exposes_type() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/ceiling_light".into(),
            device_type: "Router".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![Z2mExpose {
                    expose_type: Some("light".into()),
                    property: None,
                    features: vec![],
                }],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Light);
    }

    #[test]
    fn classify_switch_from_action_property() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "office/switch".into(),
            device_type: "EndDevice".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![Z2mExpose {
                    expose_type: None,
                    property: Some("action".into()),
                    features: vec![],
                }],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Switch);
    }

    #[test]
    fn classify_sensor_from_other_exposes() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "living_room/thermometer".into(),
            device_type: "EndDevice".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![Z2mExpose {
                    expose_type: Some("numeric".into()),
                    property: Some("temperature".into()),
                    features: vec![],
                }],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Sensor);
    }

    #[test]
    fn classify_unknown_when_no_definition() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/ceiling_light".into(),
            device_type: "Router".into(),
            definition: None,
        };
        assert_eq!(z2m.classify(), DeviceClass::Unknown);
    }

    #[test]
    fn classify_unknown_when_empty_exposes() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/ceiling_light".into(),
            device_type: "Router".into(),
            definition: Some(Z2mDefinition { exposes: vec![] }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Unknown);
    }

    #[test]
    fn classify_light_wins_over_action() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "bedroom/dimmer".into(),
            device_type: "EndDevice".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![
                    Z2mExpose {
                        expose_type: Some("light".into()),
                        property: None,
                        features: vec![],
                    },
                    Z2mExpose {
                        expose_type: None,
                        property: Some("action".into()),
                        features: vec![],
                    },
                ],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Light);
    }

    /// End-to-end: deserialize a realistic `bridge/devices` payload
    /// (light expose with nested features + dimmer with top-level
    /// `action`) and confirm classification flows through `to_device`.
    /// Pins both the wire shape we expect Z2M to send and the
    /// parse-then-classify path together.
    #[test]
    fn realistic_bridge_devices_payload_classifies() {
        let json = br#"[
            {
                "ieee_address": "0x00124b001f44b3e6",
                "friendly_name": "kitchen/ceiling_light",
                "type": "Router",
                "definition": {
                    "model": "LCT001",
                    "vendor": "Philips",
                    "description": "Hue white and color ambiance",
                    "exposes": [
                        {
                            "type": "light",
                            "features": [
                                {"type": "binary", "name": "state", "property": "state",
                                 "value_on": "ON", "value_off": "OFF"},
                                {"type": "numeric", "name": "brightness", "property": "brightness"},
                                {"type": "numeric", "name": "color_temp", "property": "color_temp"}
                            ]
                        },
                        {"type": "numeric", "name": "linkquality", "property": "linkquality"}
                    ]
                }
            },
            {
                "ieee_address": "0x00124b001f5566aa",
                "friendly_name": "kitchen/dimmer",
                "type": "EndDevice",
                "definition": {
                    "model": "324131092621",
                    "vendor": "Philips",
                    "description": "Hue dimmer switch",
                    "exposes": [
                        {"type": "enum", "name": "action", "property": "action",
                         "values": ["on_press", "off_press"]},
                        {"type": "numeric", "name": "battery", "property": "battery"},
                        {"type": "numeric", "name": "linkquality", "property": "linkquality"}
                    ]
                }
            }
        ]"#;
        let devices = parse_device_list(json).unwrap();
        assert_eq!(devices.len(), 2);

        let light = devices[0].to_device().unwrap();
        assert_eq!(light.class, DeviceClass::Light);

        let dimmer = devices[1].to_device().unwrap();
        assert_eq!(dimmer.class, DeviceClass::Switch);
    }

    /// Defensive: `expose_is_light` recurses into `features`. Verifies
    /// a hypothetical wrapper expose with a nested `light` still
    /// classifies as Light.
    #[test]
    fn classify_light_nested_under_features() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/strip".into(),
            device_type: "Router".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![Z2mExpose {
                    expose_type: Some("composite".into()),
                    property: None,
                    features: vec![Z2mExpose {
                        expose_type: Some("light".into()),
                        property: None,
                        features: vec![],
                    }],
                }],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Light);
    }

    #[test]
    fn classify_switch_when_action_nested_under_features() {
        let z2m = Z2mDevice {
            ieee_address: "0x123".into(),
            friendly_name: "kitchen/dimmer".into(),
            device_type: "EndDevice".into(),
            definition: Some(Z2mDefinition {
                exposes: vec![Z2mExpose {
                    expose_type: Some("composite".into()),
                    property: None,
                    features: vec![Z2mExpose {
                        expose_type: Some("enum".into()),
                        property: Some("action".into()),
                        features: vec![],
                    }],
                }],
            }),
        };
        assert_eq!(z2m.classify(), DeviceClass::Switch);
    }

    // ---- state parsing -------------------------------------------

    #[test]
    fn parses_full_light_state() {
        let json = br#"{
            "state": "ON",
            "brightness": 254,
            "color_temp": 250,
            "linkquality": 87
        }"#;
        let state = parse_state(json).unwrap();
        let ds = state.to_device_state();
        assert_eq!(ds.on, Some(true));
        assert_eq!(ds.brightness, Some(100));
        assert_eq!(ds.color_temp_kelvin, Some(4000));
        // linkquality is not in our model — silently ignored.
    }

    #[test]
    fn parses_partial_sensor_state() {
        let json = br#"{
            "temperature": 21.5,
            "humidity": 47.2,
            "battery": 88
        }"#;
        let state = parse_state(json).unwrap();
        let ds = state.to_device_state();
        assert_eq!(ds.temperature_celsius, Some(21.5));
        assert_eq!(ds.humidity_percent, Some(47.2));
        assert_eq!(ds.battery_percent, Some(88));
        assert_eq!(ds.on, None);
        assert_eq!(ds.brightness, None);
    }

    #[test]
    fn empty_state_is_all_none() {
        let state = parse_state(b"{}").unwrap();
        let ds = state.to_device_state();
        assert_eq!(ds, DeviceState::default());
    }

    #[test]
    fn unknown_state_string_yields_none() {
        let json = br#"{ "state": "toggle" }"#;
        let state = parse_state(json).unwrap();
        assert_eq!(state.to_device_state().on, None);
    }

    #[test]
    fn pure_action_payload_has_no_actionable_state() {
        // Z2M occasionally publishes `{"action":..,"linkquality":..}`
        // alone when battery hasn't changed since the last press —
        // no tracked field is set, so the dispatch path must skip it.
        let json = br#"{"action":"on_press","linkquality":168}"#;
        let state = parse_state(json).unwrap();
        assert!(!state.has_actionable_state_field());
    }

    #[test]
    fn brightness_only_payload_is_actionable() {
        let json = br#"{"brightness":127}"#;
        let state = parse_state(json).unwrap();
        assert!(state.has_actionable_state_field());
    }

    #[test]
    fn battery_only_is_actionable() {
        // Battery is surfaced via the HTTP API and the
        // `get_device_state` tool, so a battery-only payload from a
        // sensor must propagate through the dispatch path.
        let json = br#"{"battery":42}"#;
        let state = parse_state(json).unwrap();
        assert!(state.has_actionable_state_field());
    }

    #[test]
    fn dimmer_action_with_battery_is_actionable() {
        // The dimmer's typical payload includes battery alongside the
        // action. Even though the action itself is consumed via the
        // separate `/action` topic, the battery field must still
        // propagate so battery surfacing keeps working.
        let json = br#"{"action":"on_press","battery":100,"linkquality":168}"#;
        let state = parse_state(json).unwrap();
        assert!(state.has_actionable_state_field());
    }
}

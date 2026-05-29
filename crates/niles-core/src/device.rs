//! Device identifiers, names, and runtime state.
//!
//! Niles uses the `<room>/<device>` naming convention from upstream
//! sources (Zigbee2MQTT, etc.) as the canonical structure of the home.

use crate::error::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A normalized room name like `kitchen` or `living_room`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomName(String);

impl RoomName {
    /// Reserved prefixes for devices that don't belong to a normal room.
    pub const RESERVED: &'static [&'static str] = &["system", "outdoor", "none"];

    pub fn parse(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_segment("room", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True for `system`, `outdoor`, or `none` — devices treated as not-in-a-room.
    pub fn is_reserved(&self) -> bool {
        Self::RESERVED.contains(&self.0.as_str())
    }
}

impl fmt::Display for RoomName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A normalized device name like `ceiling_light` or `window_sensor`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceName(String);

impl DeviceName {
    pub fn parse(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_segment("device", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Names are restricted to `[a-z0-9_]+`. The explicit allowlist keeps
/// `DeviceId` round-trips bijective (no `:` or `/` to confuse the
/// parser) and rules out unicode-lookalikes in identifiers shown to
/// the LLM.
fn validate_segment(kind: &'static str, s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::InvalidName {
            kind,
            reason: "empty".into(),
        });
    }
    for c in s.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(Error::InvalidName {
                kind,
                reason: format!("'{c}' not in [a-z0-9_]"),
            });
        }
    }
    Ok(())
}

/// A source-qualified device identifier: `z2m:kitchen/ceiling_light`.
///
/// The `source` prefix is internal — it namespaces devices across
/// multiple upstream sources (Z2M, Shelly, Matter, etc.) so two
/// sources publishing the same `room/name` don't collide.
/// End users and the LLM see only the unprefixed `room/name` form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId {
    source: String,
    room: RoomName,
    name: DeviceName,
}

impl DeviceId {
    pub fn new(source: impl Into<String>, room: RoomName, name: DeviceName) -> Result<Self> {
        let source = source.into();
        validate_segment("source", &source)?;
        Ok(Self { source, room, name })
    }

    /// Parse from the `source:room/name` string form.
    pub fn parse(s: &str) -> Result<Self> {
        let (source, rest) = s
            .split_once(':')
            .ok_or_else(|| Error::InvalidDeviceId(s.to_string()))?;
        let (room, name) = rest
            .split_once('/')
            .ok_or_else(|| Error::InvalidDeviceId(s.to_string()))?;
        validate_segment("source", source)?;
        Ok(Self {
            source: source.to_string(),
            room: RoomName::parse(room)?,
            name: DeviceName::parse(name)?,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn room(&self) -> &RoomName {
        &self.room
    }

    pub fn name(&self) -> &DeviceName {
        &self.name
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}/{}", self.source, self.room, self.name)
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        DeviceId::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Current state of a device.
///
/// Each field is independently optional — a sensor reports only the
/// fields it has, a light reports `on` / `brightness` / `color_temp_kelvin`.
/// New capability fields are added here as device types are integrated.
///
/// `brightness` is normalized to 0–100 (percent). Upstream-source
/// translation (e.g. Z2M's 0–254 range, mireds → Kelvin) happens in
/// the source-specific crate, not here.
///
/// **Partial updates:** `None` means "not reported", not "off / cleared".
/// When merging an incoming partial state, copy `Some` fields over the
/// stored state — do NOT replace the whole struct, or known values get
/// clobbered by `None` for fields the upstream report didn't include.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub on: Option<bool>,
    pub brightness: Option<u8>,
    pub color_temp_kelvin: Option<u16>,
    pub temperature_celsius: Option<f32>,
    pub humidity_percent: Option<f32>,
    pub battery_percent: Option<u8>,
}

/// Classification of a device based on its upstream capabilities.
///
/// Derived at registry-population time from Z2M's `definition.exposes`
/// metadata (or equivalent for future sources). Used to distinguish
/// lights from switches and sensors without relying on runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceClass {
    Light,
    Switch,
    Sensor,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub state: DeviceState,
    pub class: DeviceClass,
    pub is_ambient: bool,
}

impl Device {
    /// Construct a new device with `is_ambient` defaulted to `false`.
    pub fn new(id: DeviceId, state: DeviceState, class: DeviceClass) -> Self {
        Self {
            id,
            state,
            class,
            is_ambient: false,
        }
    }

    /// True if this device is classified as a light.
    pub fn is_light(&self) -> bool {
        matches!(self.class, DeviceClass::Light)
    }

    /// True if this device should be driven by the ambient lighting
    /// curve and the morning routine. Ambient lights (accent /
    /// decorative lights the user controls manually) are excluded.
    pub fn is_curve_driven(&self) -> bool {
        self.is_light() && !self.is_ambient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_name_accepts_valid() {
        assert!(RoomName::parse("kitchen").is_ok());
        assert!(RoomName::parse("living_room").is_ok());
        assert!(RoomName::parse("system").is_ok());
    }

    #[test]
    fn room_name_rejects_invalid() {
        assert!(RoomName::parse("").is_err());
        assert!(RoomName::parse("Kitchen").is_err());
        assert!(RoomName::parse("living room").is_err());
        assert!(RoomName::parse("kitchen/foo").is_err());
    }

    #[test]
    fn room_name_reserved() {
        assert!(RoomName::parse("system").unwrap().is_reserved());
        assert!(RoomName::parse("outdoor").unwrap().is_reserved());
        assert!(RoomName::parse("none").unwrap().is_reserved());
        assert!(!RoomName::parse("kitchen").unwrap().is_reserved());
    }

    #[test]
    fn device_name_accepts_valid() {
        assert!(DeviceName::parse("ceiling_light").is_ok());
        assert!(DeviceName::parse("sensor_1").is_ok());
    }

    #[test]
    fn device_name_rejects_invalid() {
        assert!(DeviceName::parse("").is_err());
        assert!(DeviceName::parse("Light").is_err());
        assert!(DeviceName::parse("ceiling light").is_err());
        assert!(DeviceName::parse("ceiling/light").is_err());
    }

    /// Pin the validator's character class — `[a-z0-9_]+`, nothing more.
    /// Round-trip safety of `DeviceId` depends on `:` and `/` being rejected,
    /// and the LLM-facing identifier surface stays ASCII-only.
    #[test]
    fn segment_character_class_is_ascii_lower_digit_underscore() {
        // Accepted
        assert!(RoomName::parse("a").is_ok());
        assert!(RoomName::parse("kitchen_2").is_ok());
        assert!(RoomName::parse("_leading").is_ok());
        assert!(RoomName::parse("123").is_ok());

        // Rejected
        assert!(RoomName::parse("kitchen-light").is_err()); // hyphen
        assert!(RoomName::parse("kitchen.light").is_err()); // dot
        assert!(RoomName::parse("kitchen:light").is_err()); // colon (round-trip safety)
        assert!(RoomName::parse("é").is_err()); // non-ASCII letter
        assert!(RoomName::parse("Kitchen").is_err()); // uppercase
        assert!(RoomName::parse("kitchen light").is_err()); // space
    }

    #[test]
    fn device_id_round_trip() {
        let id = DeviceId::parse("z2m:kitchen/ceiling_light").unwrap();
        assert_eq!(id.source(), "z2m");
        assert_eq!(id.room().as_str(), "kitchen");
        assert_eq!(id.name().as_str(), "ceiling_light");
        assert_eq!(id.to_string(), "z2m:kitchen/ceiling_light");
    }

    #[test]
    fn device_id_rejects_malformed() {
        assert!(DeviceId::parse("kitchen/ceiling_light").is_err());
        assert!(DeviceId::parse("z2m:kitchen").is_err());
        assert!(DeviceId::parse("z2m:Kitchen/light").is_err());
    }

    #[test]
    fn device_id_rejects_invalid_source() {
        // Empty source: ":kitchen/light" splits to source="" — rejected.
        assert!(DeviceId::parse(":kitchen/light").is_err());
        // Uppercase / non-ASCII in source.
        assert!(DeviceId::parse("Z2M:kitchen/light").is_err());
        // new() validates too.
        let room = RoomName::parse("kitchen").unwrap();
        let name = DeviceName::parse("light").unwrap();
        assert!(DeviceId::new("", room.clone(), name.clone()).is_err());
        assert!(DeviceId::new("bad source", room, name).is_err());
    }

    #[test]
    fn device_id_source_differentiates() {
        let room = RoomName::parse("kitchen").unwrap();
        let name = DeviceName::parse("light").unwrap();
        let z2m = DeviceId::new("z2m", room.clone(), name.clone()).unwrap();
        let matter = DeviceId::new("matter", room, name).unwrap();
        assert_ne!(z2m, matter);
    }

    #[test]
    fn device_new_defaults_to_non_ambient() {
        let id = DeviceId::parse("z2m:kitchen/ceiling_light").unwrap();
        let device = Device::new(id, DeviceState::default(), DeviceClass::Light);
        assert!(!device.is_ambient);
    }

    #[test]
    fn is_curve_driven_for_normal_light() {
        let id = DeviceId::parse("z2m:kitchen/ceiling_light").unwrap();
        let device = Device::new(id, DeviceState::default(), DeviceClass::Light);
        assert!(device.is_curve_driven());
    }

    #[test]
    fn is_curve_driven_excludes_ambient_and_non_lights() {
        let id = DeviceId::parse("z2m:living_room/tv_lightstrip").unwrap();

        // Light + ambient → false
        let mut ambient_light = Device::new(id.clone(), DeviceState::default(), DeviceClass::Light);
        ambient_light.is_ambient = true;
        assert!(!ambient_light.is_curve_driven());

        // Non-light classes → false regardless of is_ambient
        for class in [
            DeviceClass::Sensor,
            DeviceClass::Switch,
            DeviceClass::Unknown,
        ] {
            let mut d = Device::new(id.clone(), DeviceState::default(), class);
            d.is_ambient = false;
            assert!(
                !d.is_curve_driven(),
                "{class:?} with is_ambient=false should not be curve_driven"
            );
            d.is_ambient = true;
            assert!(
                !d.is_curve_driven(),
                "{class:?} with is_ambient=true should not be curve_driven"
            );
        }
    }
}

//! Device identifiers, names, and runtime state.
//!
//! Niles uses the `<room>/<device>` naming convention from upstream
//! sources (Zigbee2MQTT, etc.) as the canonical structure of the home.

use crate::error::{Error, Result};
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

fn validate_segment(kind: &'static str, s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(Error::InvalidName {
            kind,
            reason: "empty".into(),
        });
    }
    if s.contains('/') {
        return Err(Error::InvalidName {
            kind,
            reason: "contains '/'".into(),
        });
    }
    if s.chars().any(|c| c.is_uppercase()) {
        return Err(Error::InvalidName {
            kind,
            reason: "uppercase forbidden".into(),
        });
    }
    if s.chars().any(char::is_whitespace) {
        return Err(Error::InvalidName {
            kind,
            reason: "whitespace forbidden".into(),
        });
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
    pub fn new(source: impl Into<String>, room: RoomName, name: DeviceName) -> Self {
        Self {
            source: source.into(),
            room,
            name,
        }
    }

    /// Parse from the `source:room/name` string form.
    pub fn parse(s: &str) -> Result<Self> {
        let (source, rest) = s
            .split_once(':')
            .ok_or_else(|| Error::InvalidDeviceId(s.to_string()))?;
        let (room, name) = rest
            .split_once('/')
            .ok_or_else(|| Error::InvalidDeviceId(s.to_string()))?;
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

/// Current state of a device.
///
/// Each field is independently optional — a sensor reports only the
/// fields it has, a light reports `on` / `brightness` / `color_temp_kelvin`.
/// New capability fields are added here as device types are integrated.
///
/// `brightness` is normalized to 0–100 (percent). Upstream-source
/// translation (e.g. Z2M's 0–254 range, mireds → Kelvin) happens in
/// the source-specific crate, not here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceState {
    pub on: Option<bool>,
    pub brightness: Option<u8>,
    pub color_temp_kelvin: Option<u16>,
    pub temperature_celsius: Option<f32>,
    pub humidity_percent: Option<f32>,
    pub battery_percent: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub state: DeviceState,
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
}

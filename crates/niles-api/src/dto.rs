//! JSON-shaped data transfer objects.
//!
//! `niles-core`'s types intentionally don't depend on `serde` —
//! they're the runtime types. This module is the *wire* shape,
//! which can evolve independently of the runtime types as the API
//! versions.

use niles_core::{Device, DeviceClass, DeviceState};
use serde::Serialize;

/// JSON shape for a device. The `id` field is the canonical
/// source-qualified identifier (e.g. `"z2m:kitchen/ceiling_light"`);
/// `source` / `room` / `name` are flattened for convenience so
/// clients don't have to re-parse `id`.
#[derive(Debug, Serialize)]
pub struct DeviceDto {
    pub id: String,
    pub source: String,
    pub room: String,
    pub name: String,
    pub class: DeviceClassDto,
    pub state: DeviceStateDto,
}

impl From<&Device> for DeviceDto {
    fn from(d: &Device) -> Self {
        Self {
            id: d.id.to_string(),
            source: d.id.source().to_string(),
            room: d.id.room().as_str().to_string(),
            name: d.id.name().as_str().to_string(),
            class: (&d.class).into(),
            state: (&d.state).into(),
        }
    }
}

/// JSON shape for a device's class. Serialized as a lowercase string
/// (`"light"`, `"switch"`, `"sensor"`, `"unknown"`) — clients should
/// treat unknown variants as forward-compatible additions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceClassDto {
    Light,
    Outlet,
    Switch,
    Sensor,
    Unknown,
}

impl From<&DeviceClass> for DeviceClassDto {
    fn from(c: &DeviceClass) -> Self {
        match c {
            DeviceClass::Light => Self::Light,
            DeviceClass::Outlet => Self::Outlet,
            DeviceClass::Switch => Self::Switch,
            DeviceClass::Sensor => Self::Sensor,
            DeviceClass::Unknown => Self::Unknown,
            // `DeviceClass` is `#[non_exhaustive]`; new upstream
            // variants serialize as `"unknown"` until this mapping
            // is updated, keeping the wire contract forward-stable.
            _ => Self::Unknown,
        }
    }
}

/// JSON shape for a device's runtime state. Missing fields are
/// serialized as `null` — same as the runtime semantic
/// ("not reported", not "off / cleared").
#[derive(Debug, Serialize)]
pub struct DeviceStateDto {
    pub on: Option<bool>,
    pub brightness: Option<u8>,
    pub color_temp_kelvin: Option<u16>,
    /// RGB color as `[r, g, b]`, each `0..=255`. Reported by color
    /// sources like WLED; `null` for devices that don't report color.
    pub rgb: Option<[u8; 3]>,
    pub temperature_celsius: Option<f32>,
    pub humidity_percent: Option<f32>,
    pub battery_percent: Option<u8>,
}

impl From<&DeviceState> for DeviceStateDto {
    fn from(s: &DeviceState) -> Self {
        Self {
            on: s.on,
            brightness: s.brightness,
            color_temp_kelvin: s.color_temp_kelvin,
            rgb: s.rgb,
            temperature_celsius: s.temperature_celsius,
            humidity_percent: s.humidity_percent,
            battery_percent: s.battery_percent,
        }
    }
}

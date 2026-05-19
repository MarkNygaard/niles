//! JSON-shaped data transfer objects.
//!
//! `niles-core`'s types intentionally don't depend on `serde` —
//! they're the runtime types. This module is the *wire* shape,
//! which can evolve independently of the runtime types as the API
//! versions.

use niles_core::{Device, DeviceState};
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
    pub state: DeviceStateDto,
}

impl From<&Device> for DeviceDto {
    fn from(d: &Device) -> Self {
        Self {
            id: d.id.to_string(),
            source: d.id.source().to_string(),
            room: d.id.room().as_str().to_string(),
            name: d.id.name().as_str().to_string(),
            state: (&d.state).into(),
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
            temperature_celsius: s.temperature_celsius,
            humidity_percent: s.humidity_percent,
            battery_percent: s.battery_percent,
        }
    }
}

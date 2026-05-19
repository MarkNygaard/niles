//! Lighting curve configuration — TOML schema + conversion to the
//! typed `niles_scheduler::CurveConfig`.
//!
//! `niles-scheduler` deliberately doesn't depend on serde; this layer
//! does the boundary translation. Times are encoded as `"HH:MM"`
//! strings in TOML and parsed via `MinuteOfDay::from_str`.

use crate::error::{Error, Result};
use niles_scheduler::{CurveConfig, MinuteOfDay};
use serde::Deserialize;

/// `[lighting]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightingConfig {
    pub morning_start: String,
    pub morning_end: String,
    pub sunset_start: String,
    pub sunset_end: String,
    pub night_floor_brightness: u8,
    pub daytime_brightness: u8,
    pub color_temp_anchors: Vec<ColorTempAnchor>,
}

/// One row of `[[lighting.color_temp_anchors]]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorTempAnchor {
    /// `"HH:MM"` time of day.
    pub time: String,
    /// Color temperature in Kelvin.
    pub kelvin: u16,
}

impl LightingConfig {
    /// Parse times and anchors, then validate the resulting `CurveConfig`.
    pub fn to_curve_config(&self) -> Result<CurveConfig> {
        let mut anchors = Vec::with_capacity(self.color_temp_anchors.len());
        for anchor in &self.color_temp_anchors {
            anchors.push((parse_time(&anchor.time)?, anchor.kelvin));
        }

        let curve = CurveConfig {
            morning_start: parse_time(&self.morning_start)?,
            morning_end: parse_time(&self.morning_end)?,
            sunset_start: parse_time(&self.sunset_start)?,
            sunset_end: parse_time(&self.sunset_end)?,
            night_floor_brightness: self.night_floor_brightness,
            daytime_brightness: self.daytime_brightness,
            color_temp_anchors: anchors,
        };

        curve.validate().map_err(|e| Error::InvalidSection {
            section: "lighting",
            reason: e.to_string(),
        })?;

        Ok(curve)
    }
}

fn parse_time(s: &str) -> Result<MinuteOfDay> {
    s.parse()
        .map_err(|e: niles_scheduler::Error| Error::InvalidSection {
            section: "lighting",
            reason: format!("invalid time '{s}': {e}"),
        })
}

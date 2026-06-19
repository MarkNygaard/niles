//! Lighting curve configuration — TOML schema + conversion to the
//! typed `niles_scheduler::CurveConfig`.
//!
//! `niles-scheduler` deliberately doesn't depend on serde; this layer
//! does the boundary translation. Times are encoded as `"HH:MM"`
//! strings in TOML and parsed via `MinuteOfDay::from_str`.

use crate::error::{Error, Result};
use chrono::{NaiveDate, Weekday};
use niles_core::DeviceId;
use niles_scheduler::{CurveConfig, MinuteOfDay, MorningRoutineConfig};
use serde::Deserialize;
use std::str::FromStr;

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
    pub morning_routine: Option<MorningRoutineConfigDto>,
}

/// `[lighting.morning_routine]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorningRoutineConfigDto {
    pub fire_days: Vec<String>,
    /// Devices to wake up. **Omit (or leave empty) to target every
    /// curve-managed light** (all non-ambient lights, honoring
    /// `[ambient_lights]`). When set, each entry must be a fully
    /// qualified device id (e.g. `wled:living_room/ceiling`).
    #[serde(default)]
    pub target_devices: Vec<String>,
    /// Lights to exclude, applied after `target_devices` resolves — so
    /// an empty `target_devices` plus `exclude_devices` means "all
    /// lights except these". Fully qualified ids.
    #[serde(default)]
    pub exclude_devices: Vec<String>,
    #[serde(default)]
    pub skip_overrides: Vec<String>,
}

impl MorningRoutineConfigDto {
    /// Parse strings into typed `MorningRoutineConfig`, validating
    /// every field.
    pub fn to_morning_routine_config(&self) -> Result<MorningRoutineConfig> {
        let fire_days = self
            .fire_days
            .iter()
            .map(|s| parse_weekday(s))
            .collect::<Result<Vec<_>>>()?;
        let target_devices = self
            .target_devices
            .iter()
            .map(|s| parse_device_id(s))
            .collect::<Result<Vec<_>>>()?;
        let exclude_devices = self
            .exclude_devices
            .iter()
            .map(|s| parse_device_id(s))
            .collect::<Result<Vec<_>>>()?;
        let skip_overrides = self
            .skip_overrides
            .iter()
            .map(|s| parse_naive_date(s))
            .collect::<Result<Vec<_>>>()?;

        Ok(MorningRoutineConfig {
            fire_days,
            target_devices,
            exclude_devices,
            skip_overrides,
        })
    }
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
        let anchors = self
            .color_temp_anchors
            .iter()
            .map(|a| Ok((parse_time(&a.time)?, a.kelvin)))
            .collect::<Result<Vec<_>>>()?;

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

fn parse_weekday(s: &str) -> Result<Weekday> {
    let lowered = s.to_ascii_lowercase();
    Weekday::from_str(&lowered).map_err(|e| Error::InvalidSection {
        section: "lighting",
        reason: format!("invalid weekday '{s}': {e}"),
    })
}

fn parse_device_id(s: &str) -> Result<DeviceId> {
    DeviceId::parse(s).map_err(|e| Error::InvalidSection {
        section: "lighting",
        reason: format!("invalid target_device '{s}': {e}"),
    })
}

fn parse_naive_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| Error::InvalidSection {
        section: "lighting",
        reason: format!("invalid skip_override date '{s}': {e}"),
    })
}

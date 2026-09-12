//! Lighting curve configuration — TOML schema + conversion to the
//! typed `niles_scheduler::CurveConfig`.
//!
//! `niles-scheduler` deliberately doesn't depend on serde; this layer
//! does the boundary translation. Times are encoded as `"HH:MM"`
//! strings in TOML and parsed via `MinuteOfDay::from_str`.

use crate::error::{Error, Result};
use chrono::{NaiveDate, Weekday};
use niles_core::DeviceId;
use niles_scheduler::{CurveConfig, CurvePause, MinuteOfDay, MorningRoutineConfig, WeekInstant};
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
    /// Optional recurring weekly window during which the curve freezes
    /// at the level it held when the window opened — so lights switched
    /// on mid-window land on that level too. Both endpoints are
    /// `"<weekday> HH:MM"` (e.g. `"fri 12:00"`) and must be set together.
    /// Brightness for lights listed in `[ambient_lights]`, which sit out
    /// the curve. Omit to leave them wherever they were — the behaviour
    /// before this existed.
    ///
    /// These live here rather than in `[ambient_lights]` so that one
    /// section holds everything about how lights are driven, and the
    /// other holds only which lights are exempt. Both are re-read on
    /// every tick.
    #[serde(default)]
    pub ambient_brightness: Option<u8>,
    /// Colour temperature for ambient lights, in Kelvin. Low is the point
    /// — 2000-2200 K is candle-to-lamp warm.
    ///
    /// Only useful for a light that has a white channel. An RGB strip
    /// can't act on it at all, and WLED never even receives it — use
    /// [`Self::ambient_color`] for those.
    #[serde(default)]
    pub ambient_kelvin: Option<u16>,
    /// Colour for ambient lights as `#rrggbb`. Takes precedence over
    /// [`Self::ambient_kelvin`]: a light is in colour mode or white
    /// mode, never both, so sending each would leave the winner up to
    /// the firmware.
    #[serde(default)]
    pub ambient_color: Option<String>,
    #[serde(default)]
    pub curve_pause_start: Option<String>,
    #[serde(default)]
    pub curve_pause_end: Option<String>,
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
    /// What ambient lights are held at, if anything is configured.
    /// `None` for a field means "leave that alone".
    pub fn ambient_target(&self) -> Option<AmbientTarget> {
        let rgb = self.ambient_color.as_deref().and_then(parse_hex_color);
        let target = AmbientTarget {
            brightness: self.ambient_brightness,
            // An explicit colour is the more specific ask, and a light
            // cannot be in both modes at once.
            kelvin: if rgb.is_some() {
                None
            } else {
                self.ambient_kelvin
            },
            rgb,
        };
        (target != AmbientTarget::default()).then_some(target)
    }

    /// Parse times and anchors, then validate the resulting `CurveConfig`.
    pub fn to_curve_config(&self) -> Result<CurveConfig> {
        let anchors = self
            .color_temp_anchors
            .iter()
            .map(|a| Ok((parse_time(&a.time)?, a.kelvin)))
            .collect::<Result<Vec<_>>>()?;

        let pause = match (&self.curve_pause_start, &self.curve_pause_end) {
            (Some(s), Some(e)) => Some(CurvePause {
                start: parse_week_instant(s)?,
                end: parse_week_instant(e)?,
            }),
            (None, None) => None,
            _ => {
                return Err(Error::InvalidSection {
                    section: "lighting",
                    reason:
                        "curve_pause_start and curve_pause_end must both be set or both omitted"
                            .into(),
                });
            }
        };

        let curve = CurveConfig {
            morning_start: parse_time(&self.morning_start)?,
            morning_end: parse_time(&self.morning_end)?,
            sunset_start: parse_time(&self.sunset_start)?,
            sunset_end: parse_time(&self.sunset_end)?,
            night_floor_brightness: self.night_floor_brightness,
            daytime_brightness: self.daytime_brightness,
            color_temp_anchors: anchors,
            pause,
        };

        curve.validate().map_err(|e| Error::InvalidSection {
            section: "lighting",
            reason: e.to_string(),
        })?;

        if let Some(brightness) = self.ambient_brightness
            && brightness > 100
        {
            return Err(Error::InvalidSection {
                section: "lighting",
                reason: format!("ambient_brightness {brightness} is above 100"),
            });
        }
        if let Some(kelvin) = self.ambient_kelvin
            && !(1000..=10000).contains(&kelvin)
        {
            return Err(Error::InvalidSection {
                section: "lighting",
                reason: format!("ambient_kelvin {kelvin}K is outside 1000..=10000"),
            });
        }
        if let Some(color) = &self.ambient_color
            && parse_hex_color(color).is_none()
        {
            return Err(Error::InvalidSection {
                section: "lighting",
                reason: format!("ambient_color {color:?} is not a #rrggbb colour"),
            });
        }

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

/// The fixed setting ambient lights are held at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AmbientTarget {
    pub brightness: Option<u8>,
    pub kelvin: Option<u16>,
    pub rgb: Option<[u8; 3]>,
}

/// `#rrggbb` (or bare `rrggbb`) to channels. `None` for anything else.
pub fn parse_hex_color(raw: &str) -> Option<[u8; 3]> {
    let hex = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Parse a `"<weekday> HH:MM"` string (e.g. `"fri 12:00"`) into a
/// `WeekInstant`.
fn parse_week_instant(s: &str) -> Result<WeekInstant> {
    let mut parts = s.split_whitespace();
    let (Some(day), Some(time), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(Error::InvalidSection {
            section: "lighting",
            reason: format!("expected '<weekday> HH:MM', got '{s}'"),
        });
    };
    Ok(WeekInstant::new(parse_weekday(day)?, parse_time(time)?))
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

#[cfg(test)]
mod ambient_tests {
    use super::*;

    fn lighting_with(extra: &str) -> LightingConfig {
        let base = r#"
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#;
        toml::from_str(&format!("{extra}{base}")).expect("fixture parses")
    }

    #[test]
    fn a_colour_is_read_as_channels() {
        let cfg = lighting_with(
            "ambient_color = \"#ff8000\"
",
        );
        assert_eq!(cfg.ambient_target().unwrap().rgb, Some([255, 128, 0]));
    }

    #[test]
    fn a_colour_wins_over_a_colour_temperature() {
        // A light is in colour mode or white mode, never both. Sending
        // each would leave the winner up to the firmware.
        let cfg = lighting_with(
            "ambient_kelvin = 2200
ambient_color = \"#ff8000\"
",
        );
        let target = cfg.ambient_target().unwrap();
        assert_eq!(target.rgb, Some([255, 128, 0]));
        assert_eq!(target.kelvin, None);
    }

    #[test]
    fn a_colour_that_is_not_one_is_rejected() {
        let err = lighting_with(
            "ambient_color = \"burnt orange\"
",
        )
        .to_curve_config()
        .expect_err("not a colour");
        assert!(err.to_string().contains("ambient_color"), "{err}");
    }

    #[test]
    fn no_ambient_settings_means_no_ambient_target() {
        // The behaviour before this existed: ambient lights sit out the
        // curve and are otherwise left exactly as they were.
        assert_eq!(lighting_with("").ambient_target(), None);
    }

    #[test]
    fn brightness_alone_is_a_target() {
        // Dim it, but leave whatever colour it is showing.
        let cfg = lighting_with("ambient_brightness = 25\n");
        let target = cfg.ambient_target().unwrap();
        assert_eq!(target.brightness, Some(25));
        assert_eq!((target.kelvin, target.rgb), (None, None));
        cfg.to_curve_config().expect("valid");
    }

    #[test]
    fn brightness_and_kelvin_together() {
        let cfg = lighting_with("ambient_brightness = 25\nambient_kelvin = 2200\n");
        let target = cfg.ambient_target().unwrap();
        assert_eq!((target.brightness, target.kelvin), (Some(25), Some(2200)));
        cfg.to_curve_config().expect("valid");
    }

    #[test]
    fn brightness_above_100_is_rejected() {
        let err = lighting_with("ambient_brightness = 120\n")
            .to_curve_config()
            .expect_err("out of range");
        assert!(err.to_string().contains("ambient_brightness"), "{err}");
    }

    #[test]
    fn kelvin_outside_the_sane_range_is_rejected() {
        let err = lighting_with("ambient_kelvin = 500\n")
            .to_curve_config()
            .expect_err("out of range");
        assert!(err.to_string().contains("ambient_kelvin"), "{err}");
    }
}

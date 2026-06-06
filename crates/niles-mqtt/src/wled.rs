//! WLED message types and conversions to `niles-core` types.
//!
//! WLED publishes plain-text (not JSON) on a handful of MQTT topics:
//!
//! - `<topic>/g` — integer brightness `0..=255`
//! - `<topic>/c` — hex color `#RRGGBB`
//! - `<topic>/status` — `"online"` / `"offline"`
//!
//! Commands are sent to `<topic>/api` as WLED JSON.

use niles_core::DeviceState;
use serde::Serialize;

/// WLED brightness `0..=255` → percent `0..=100`, rounded to nearest.
pub fn wled_brightness_to_percent(v: u8) -> u8 {
    ((u16::from(v) * 100 + 127) / 255) as u8
}

/// Percent `0..=100` → WLED brightness `0..=255`, rounded to nearest.
/// Clamps inputs > 100 defensively.
pub fn percent_to_wled_brightness(pct: u8) -> u8 {
    ((u16::from(pct.min(100)) * 255 + 50) / 100) as u8
}

/// Parse a hex color string. Accepts `#RRGGBB` or `RRGGBB`, case-insensitive.
pub fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 {
        return None;
    }
    let mut out = [0u8; 3];
    for (i, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let byte = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
        out[i] = byte;
    }
    Some(out)
}

/// Parse the body of a `<topic>/g` message — integer brightness `0..=255`.
/// Derives `on` from brightness (> 0 → true) so the ambient curve picks
/// WLED up as a light.
pub fn parse_g(payload: &[u8]) -> Option<DeviceState> {
    let s = std::str::from_utf8(payload).ok()?;
    let v: u8 = s.trim().parse().ok()?;
    let pct = wled_brightness_to_percent(v);
    Some(DeviceState {
        on: Some(pct > 0),
        brightness: Some(pct),
        ..Default::default()
    })
}

/// Parse the body of a `<topic>/c` message — hex color.
pub fn parse_c(payload: &[u8]) -> Option<DeviceState> {
    let s = std::str::from_utf8(payload).ok()?;
    let rgb = parse_hex_color(s)?;
    Some(DeviceState {
        rgb: Some(rgb),
        ..Default::default()
    })
}

/// Parse the body of a `<topic>/status` message.
pub fn parse_status(payload: &[u8]) -> Option<bool> {
    match payload {
        b"online" => Some(true),
        b"offline" => Some(false),
        _ => None,
    }
}

/// Build the topic + JSON payload for a WLED `/api` command.
///
/// Only `on`, `brightness`, and `rgb` are mapped. `color_temp_kelvin` is
/// ignored — RGB strips have no natural color-temperature mapping.
///
/// Returns `None` if none of the mapped fields is set (no-op guard).
pub fn format_wled_command(base_topic: &str, target: &DeviceState) -> Option<(String, String)> {
    if target.on.is_none() && target.brightness.is_none() && target.rgb.is_none() {
        return None;
    }
    let json = serde_json::to_string(&WledApiPayload {
        on: target.on,
        bri: target.brightness.map(percent_to_wled_brightness),
        seg: target.rgb.map(|rgb| vec![WledSegment { col: vec![rgb] }]),
    })
    .ok()?;
    Some((format!("{base_topic}/api"), json))
}

#[derive(Debug, Serialize)]
struct WledApiPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bri: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seg: Option<Vec<WledSegment>>,
}

#[derive(Debug, Serialize)]
struct WledSegment {
    col: Vec<[u8; 3]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_round_trip() {
        assert_eq!(wled_brightness_to_percent(0), 0);
        assert_eq!(wled_brightness_to_percent(255), 100);
        assert_eq!(percent_to_wled_brightness(0), 0);
        assert_eq!(percent_to_wled_brightness(100), 255);
        // Approximate round-trip
        assert_eq!(
            wled_brightness_to_percent(percent_to_wled_brightness(50)),
            50
        );
    }

    #[test]
    fn percent_clamps_above_100() {
        assert_eq!(percent_to_wled_brightness(200), 255);
    }

    #[test]
    fn parse_hex_color_cases() {
        assert_eq!(parse_hex_color("#FF8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_color("ff8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_color("00FF00"), Some([0, 255, 0]));
        assert_eq!(parse_hex_color("bad"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
    }

    #[test]
    fn parse_g_cases() {
        let st = parse_g(b"128").unwrap();
        assert_eq!(st.brightness, Some(50));
        assert_eq!(st.on, Some(true));

        let st0 = parse_g(b"0").unwrap();
        assert_eq!(st0.brightness, Some(0));
        assert_eq!(st0.on, Some(false));

        assert!(parse_g(b"abc").is_none());
    }

    #[test]
    fn parse_c_cases() {
        let st = parse_c(b"#00FF00").unwrap();
        assert_eq!(st.rgb, Some([0, 255, 0]));
        assert!(parse_c(b"junk").is_none());
    }

    #[test]
    fn parse_status_cases() {
        assert_eq!(parse_status(b"online"), Some(true));
        assert_eq!(parse_status(b"offline"), Some(false));
        assert_eq!(parse_status(b"unknown"), None);
    }

    #[test]
    fn format_wled_all_fields() {
        let target = DeviceState {
            on: Some(true),
            brightness: Some(50),
            rgb: Some([255, 128, 0]),
            ..Default::default()
        };
        let (topic, payload) = format_wled_command("wled/office", &target).unwrap();
        assert_eq!(topic, "wled/office/api");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["on"], true);
        assert_eq!(v["bri"], 128);
        assert_eq!(v["seg"][0]["col"][0], json!([255, 128, 0]));
    }

    #[test]
    fn format_wled_on_only() {
        let target = DeviceState {
            on: Some(false),
            ..Default::default()
        };
        let (topic, payload) = format_wled_command("wled/office", &target).unwrap();
        assert_eq!(topic, "wled/office/api");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["on"], false);
        assert!(!v.as_object().unwrap().contains_key("bri"));
        assert!(!v.as_object().unwrap().contains_key("seg"));
    }

    #[test]
    fn format_wled_empty_returns_none() {
        assert!(format_wled_command("wled/office", &DeviceState::default()).is_none());
    }

    #[test]
    fn format_wled_color_temp_only_returns_none() {
        let target = DeviceState {
            color_temp_kelvin: Some(3000),
            ..Default::default()
        };
        assert!(format_wled_command("wled/office", &target).is_none());
    }

    use serde_json::json;
}

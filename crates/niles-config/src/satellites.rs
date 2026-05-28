//! Satellite configuration section.

use crate::error::{Error, Result};
use niles_core::RoomName;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::IpAddr;

/// `[satellites]` section of the config file.
///
/// Optional. Maps satellite LAN IPs to canonical room names so the
/// Tier 1 LLM can know which room a user is speaking from.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(transparent)]
pub struct SatellitesConfig {
    pub satellites: HashMap<String, SatelliteConfig>,
}

/// Per-satellite entry inside `[satellites.<name>]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SatelliteConfig {
    pub ip: String,
    pub room: String,
}

impl SatellitesConfig {
    pub fn validate(&self) -> Result<()> {
        let mut seen_ips = HashMap::new();
        for (name, sat) in &self.satellites {
            if name.trim().is_empty() {
                return Err(Error::InvalidSection {
                    section: "satellites",
                    reason: "satellite name must not be empty".into(),
                });
            }
            let ip = sat
                .ip
                .parse::<IpAddr>()
                .map_err(|_| Error::InvalidSection {
                    section: "satellites",
                    reason: format!(
                        "satellites.{name}.ip = {:?} is not a valid IP address",
                        sat.ip
                    ),
                })?;
            if let Some(prev) = seen_ips.insert(ip, name.as_str()) {
                return Err(Error::InvalidSection {
                    section: "satellites",
                    reason: format!("satellites.{name}.ip = {ip} duplicates satellites.{prev}.ip"),
                });
            }
            if RoomName::parse(&sat.room).is_err() {
                return Err(Error::InvalidSection {
                    section: "satellites",
                    reason: format!(
                        "satellites.{name}.room = {:?} is not a valid canonical room name",
                        sat.room
                    ),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn valid_toml() -> &'static str {
        r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[mqtt]
host = "192.168.42.16"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

[wyoming]
bind_address = "0.0.0.0:10300"

[stt]
api_key_env = "GROQ_API_KEY"

[tts]

[llm]
api_key_env = "GROQ_API_KEY"

[lighting]
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "05:45"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "06:30"
kelvin = 2700

[[lighting.color_temp_anchors]]
time = "12:00"
kelvin = 4500

[[lighting.color_temp_anchors]]
time = "21:30"
kelvin = 2700

[[lighting.color_temp_anchors]]
time = "23:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#
    }

    #[test]
    fn parses_valid_section() {
        let toml = format!(
            r#"{}
[satellites.dev_atom]
ip = "192.168.42.5"
room = "living_room"
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.satellites.satellites.len(), 1);
        let sat = cfg.satellites.satellites.get("dev_atom").unwrap();
        assert_eq!(sat.ip, "192.168.42.5");
        assert_eq!(sat.room, "living_room");
    }

    #[test]
    fn absent_section_defaults_to_empty() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert!(cfg.satellites.satellites.is_empty());
    }

    #[test]
    fn rejects_invalid_ip() {
        let toml = format!(
            r#"{}
[satellites.dev_atom]
ip = "not-an-ip"
room = "living_room"
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidSection {
                    section: "satellites",
                    ..
                }
            ),
            "expected InvalidSection {{ section: satellites }}, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("not-an-ip"),
            "error should contain offending value: {msg}"
        );
    }

    #[test]
    fn rejects_invalid_room() {
        let toml = format!(
            r#"{}
[satellites.dev_atom]
ip = "192.168.42.5"
room = "Living Room"
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidSection {
                    section: "satellites",
                    ..
                }
            ),
            "expected InvalidSection {{ section: satellites }}, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("Living Room"),
            "error should contain offending value: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_satellite_field() {
        let toml = format!(
            r#"{}
[satellites.dev_atom]
ipp = "192.168.42.5"
room = "living_room"
"#,
            valid_toml().trim_end_matches('\n')
        );
        assert!(Config::load_from_str(&toml).is_err());
    }

    #[test]
    fn rejects_empty_room() {
        let toml = format!(
            r#"{}
[satellites.dev_atom]
ip = "192.168.42.5"
room = ""
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidSection {
                    section: "satellites",
                    ..
                }
            ),
            "expected InvalidSection {{ section: satellites }}, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("room"),
            "error should mention room field: {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_ip() {
        let toml = format!(
            r#"{}
[satellites.living_room_sat]
ip = "192.168.42.5"
room = "living_room"

[satellites.kitchen_sat]
ip = "192.168.42.5"
room = "kitchen"
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidSection {
                    section: "satellites",
                    ..
                }
            ),
            "expected InvalidSection {{ section: satellites }}, got {err:?}"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("192.168.42.5"),
            "error should contain offending IP: {msg}"
        );
        assert!(
            msg.contains("duplicates") || msg.contains("duplicate"),
            "error should mention duplication: {msg}"
        );
    }
}

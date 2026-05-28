//! Speaker configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::HashMap;

/// `[speakers]` section of the config file.
///
/// Optional. If absent, no speakers are registered and voice
/// commands for media control fall through to Tier 1.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SpeakersConfig {
    #[serde(default)]
    pub rooms: HashMap<String, SpeakerConfig>,
}

/// Per-room speaker entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerConfig {
    pub ip: String,
    #[serde(default = "default_speaker_type")]
    pub kind: String,
}

fn default_speaker_type() -> String {
    "sonos".into()
}

impl SpeakersConfig {
    pub fn validate(&self) -> Result<()> {
        for (room, sp) in &self.rooms {
            if room.trim().is_empty() {
                return Err(Error::InvalidSection {
                    section: "speakers",
                    reason: "room key must not be empty".into(),
                });
            }
            if sp.ip.trim().is_empty() {
                return Err(Error::InvalidSection {
                    section: "speakers",
                    reason: format!("speakers.{room}.ip must not be empty"),
                });
            }
            if sp.kind != "sonos" {
                return Err(Error::InvalidSection {
                    section: "speakers",
                    reason: format!(
                        "speakers.{room}.kind '{}' is not supported (only 'sonos')",
                        sp.kind
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

    #[test]
    fn default_empty_parses() {
        let cfg: SpeakersConfig = toml::from_str("").unwrap();
        assert!(cfg.rooms.is_empty());
    }

    #[test]
    fn single_room_ok() {
        let cfg: SpeakersConfig = toml::from_str(
            r#"
            [rooms.living_room]
            ip = "192.168.69.174"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.rooms.len(), 1);
        assert_eq!(cfg.rooms["living_room"].ip, "192.168.69.174");
        assert_eq!(cfg.rooms["living_room"].kind, "sonos");
        cfg.validate().unwrap();
    }

    #[test]
    fn empty_ip_rejected() {
        let cfg: SpeakersConfig = toml::from_str(
            r#"
            [rooms.kitchen]
            ip = ""
            "#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn empty_room_key_rejected() {
        // TOML doesn't support empty keys in inline tables, but we
        // can simulate the struct state directly.
        let mut rooms = HashMap::new();
        rooms.insert(
            "".into(),
            SpeakerConfig {
                ip: "192.168.69.174".into(),
                kind: "sonos".into(),
            },
        );
        let cfg = SpeakersConfig { rooms };
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("room key must not be empty"), "{msg}");
    }

    #[test]
    fn unsupported_kind_rejected() {
        let cfg: SpeakersConfig = toml::from_str(
            r#"
            [rooms.kitchen]
            ip = "192.168.69.174"
            kind = "bluetooth"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("bluetooth"), "{msg}");
    }

    #[test]
    fn unknown_field_rejected() {
        let result: std::result::Result<SpeakersConfig, _> = toml::from_str(
            r#"
            [rooms.kitchen]
            ip = "192.168.69.174"
            port = 1400
            "#,
        );
        assert!(result.is_err());
    }
}

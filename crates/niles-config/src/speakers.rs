//! Speaker configuration section.

use crate::error::{Error, Result};
use niles_core::RoomName;
use serde::Deserialize;
use std::collections::HashMap;

/// `[speakers]` section of the config file: the Sonos household, and
/// which Niles room each of its rooms is in.
///
/// Optional. Without a `host`, no speaker is known and media commands
/// say there is none in the room.
///
/// ```toml
/// [speakers]
/// host = "192.168.10.174"
///
/// [speakers.sonos.RINCON_5CAAFD1FD8C601400]
/// room = "living_room"
/// name = "Living Room"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakersConfig {
    /// Off keeps the address and the rooms without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Any one Sonos. Each of them can describe the whole household,
    /// so one address is enough, and it does not have to be the same
    /// speaker that plays.
    #[serde(default)]
    pub host: String,
    /// Sonos rooms by Sonos's own id (`RINCON_…`), never by address:
    /// DHCP moves addresses, the id stays with the speaker.
    #[serde(default)]
    pub sonos: HashMap<String, SonosSpeaker>,
}

impl Default for SpeakersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: String::new(),
            sonos: HashMap::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// `[speakers.sonos.<id>]` — one Sonos room, placed in a Niles room.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SonosSpeaker {
    pub room: String,
    /// What the Sonos app called it when it was placed, so a speaker
    /// that does not answer is still listed by a name, not an id.
    #[serde(default)]
    pub name: String,
}

impl SpeakersConfig {
    /// Whether there is a Sonos to ask: switched on, with an address.
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.host.trim().is_empty()
    }

    /// The Sonos ids placed in `room`.
    pub fn in_room<'a>(&'a self, room: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.sonos
            .iter()
            .filter(move |(_, speaker)| speaker.room == room)
            .map(|(id, _)| id.as_str())
    }

    pub fn validate(&self) -> Result<()> {
        for (id, speaker) in &self.sonos {
            if id.trim().is_empty() {
                return Err(invalid("a Sonos id must not be empty".into()));
            }
            RoomName::parse(&speaker.room).map_err(|e| {
                invalid(format!(
                    "speakers.sonos.{id}.room = {:?}: {e}",
                    speaker.room
                ))
            })?;
        }
        Ok(())
    }
}

fn invalid(reason: String) -> Error {
    Error::InvalidSection {
        section: "speakers",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> SpeakersConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn nothing_written_is_no_speakers() {
        let cfg = parse("");
        assert!(!cfg.is_configured());
        assert!(cfg.sonos.is_empty());
        cfg.validate().unwrap();
    }

    #[test]
    fn an_address_is_what_makes_it_configured() {
        assert!(parse(r#"host = "192.168.10.174""#).is_configured());
        assert!(!parse("host = \"192.168.10.174\"\nenabled = false").is_configured());
    }

    #[test]
    fn places_sonos_rooms_in_niles_rooms() {
        let cfg = parse(
            r#"
            host = "192.168.10.174"
            [sonos.RINCON_BAR]
            room = "living_room"
            name = "Living Room"
            [sonos.RINCON_BACK]
            room = "living_room"
            [sonos.RINCON_MOVE]
            room = "kitchen"
            "#,
        );
        cfg.validate().unwrap();
        let mut living: Vec<_> = cfg.in_room("living_room").collect();
        living.sort();
        assert_eq!(living, ["RINCON_BACK", "RINCON_BAR"]);
        assert_eq!(cfg.in_room("bedroom").count(), 0);
    }

    #[test]
    fn rejects_a_room_no_device_could_have() {
        let cfg = parse(
            r#"
            [sonos.RINCON_BAR]
            room = "Living Room"
            "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("speakers.sonos.RINCON_BAR.room"), "{err}");
    }

    #[test]
    fn the_old_room_by_address_shape_is_refused() {
        // A speaker by IP per room was the first slice. Addresses move,
        // so it is gone rather than half-supported.
        let old = toml::from_str::<SpeakersConfig>(
            r#"
            [rooms.kitchen]
            ip = "192.168.10.174"
            "#,
        );
        assert!(old.is_err());
    }
}

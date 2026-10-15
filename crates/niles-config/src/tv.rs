//! TV configuration section.

use crate::error::{Error, Result};
use niles_core::RoomName;
use serde::Deserialize;

/// `[tv]` section of the config file: an LG webOS TV.
///
/// Optional. Without a `host` there is no TV, and "turn on the TV" is
/// answered as such. The key the TV issued at pairing is a credential
/// (`tv.client_key`), not part of this section.
///
/// ```toml
/// [tv]
/// host = "192.168.69.10"
/// mac = "a8:23:fe:01:02:03"
/// room = "living_room"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvConfig {
    /// Off keeps the address and the pairing without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    /// The MAC the TV is woken by. Read from the TV at pairing, so
    /// rarely typed; one typed is kept through a pairing again.
    #[serde(default)]
    pub mac: String,
    /// Every MAC the TV reported at its last pairing, the one Niles
    /// chose for `mac` first. Each gets a magic packet too, and the
    /// first is how a `mac` somebody typed is told from one Niles found.
    #[serde(default)]
    pub detected_macs: Vec<String>,
    /// Where it stands: "the TV" said in this room is this one, and so
    /// is the room's soundbar it plays through.
    #[serde(default)]
    pub room: String,
    /// Niles's announcements — a delivery on its way, a timer done —
    /// also shown on the screen while it is on.
    #[serde(default = "default_true")]
    pub show_announcements: bool,
    /// The variable holding the pairing key, when it comes from one
    /// rather than from Niles's own store.
    #[serde(default)]
    pub client_key_env: String,
}

impl Default for TvConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: String::new(),
            mac: String::new(),
            detected_macs: Vec::new(),
            room: String::new(),
            show_announcements: true,
            client_key_env: String::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl TvConfig {
    /// Whether there is a TV to reach: switched on, with an address.
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.host.trim().is_empty()
    }

    /// The key the TV issued when it was paired.
    pub fn resolve_client_key(&self) -> Result<String> {
        crate::env::require_secret("tv", "tv.client_key", &self.client_key_env)
    }

    /// Every MAC a magic packet goes to: `mac`, then each the TV
    /// reported at pairing, once each.
    pub fn wake_macs(&self) -> Vec<String> {
        let mut macs: Vec<String> = Vec::new();
        for mac in std::iter::once(&self.mac).chain(&self.detected_macs) {
            let mac = mac.trim();
            if !mac.is_empty() && !macs.iter().any(|m| same_mac(m, mac)) {
                macs.push(mac.to_string());
            }
        }
        macs
    }

    /// The MAC Niles chose at the last pairing.
    pub fn detected_mac(&self) -> Option<&str> {
        self.detected_macs
            .first()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty())
    }

    /// Whether `mac` is one somebody set rather than the one pairing
    /// found, and so is kept when the TV is paired again. One from
    /// before Niles kept what it found counts as set: it may have been.
    pub fn mac_set_by_hand(&self) -> bool {
        let mac = self.mac.trim();
        !mac.is_empty() && !self.detected_mac().is_some_and(|d| same_mac(d, mac))
    }

    pub fn validate(&self) -> Result<()> {
        if !self.room.is_empty() {
            RoomName::parse(&self.room)
                .map_err(|e| invalid(format!("tv.room = {:?}: {e}", self.room)))?;
        }
        let mac = self.mac.trim();
        if !mac.is_empty() && !well_formed(mac) {
            return Err(invalid(format!("tv.mac = {mac:?} is not a MAC address")));
        }
        if let Some(mac) = self.detected_macs.iter().find(|m| !well_formed(m.trim())) {
            return Err(invalid(format!(
                "tv.detected_macs has {mac:?}, which is not a MAC address"
            )));
        }
        Ok(())
    }
}

fn well_formed(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split([':', '-']).collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok())
}

/// One MAC however it is written: "A8-23-FE-01-02-03" is "a8:23:fe:01:02:03".
fn same_mac(a: &str, b: &str) -> bool {
    let digits = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    digits(a) == digits(b)
}

fn invalid(reason: String) -> Error {
    Error::InvalidSection {
        section: "tv",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> TvConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn nothing_written_is_no_tv() {
        let cfg = parse("");
        assert!(!cfg.is_configured());
        assert!(cfg.show_announcements);
        cfg.validate().unwrap();
    }

    #[test]
    fn an_address_makes_a_tv() {
        let cfg = parse(
            r#"
            host = "192.168.69.10"
            mac = "A8:23:FE:01:02:03"
            room = "living_room"
            "#,
        );
        assert!(cfg.is_configured());
        cfg.validate().unwrap();
    }

    #[test]
    fn refuses_a_mac_that_is_not_one() {
        let err = parse(r#"mac = "a8:23:fe""#)
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("tv.mac"), "{err}");
    }

    #[test]
    fn refuses_a_detected_mac_that_is_not_one() {
        let err = parse(r#"detected_macs = ["a8:23:fe:01:02:03", "nope"]"#)
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("tv.detected_macs"), "{err}");
    }

    #[test]
    fn the_mac_set_is_woken_first_then_every_one_detected_once() {
        let cfg = parse(
            r#"
            mac = "A8-23-FE-01-02-03"
            detected_macs = ["64:e4:a5:6c:ab:b4", "a8:23:fe:01:02:03"]
            "#,
        );
        assert_eq!(cfg.wake_macs(), ["A8-23-FE-01-02-03", "64:e4:a5:6c:ab:b4"]);
        let cfg = parse(r#"detected_macs = ["64:e4:a5:6c:ab:b4"]"#);
        assert_eq!(cfg.wake_macs(), ["64:e4:a5:6c:ab:b4"]);
        assert!(parse("").wake_macs().is_empty());
    }

    #[test]
    fn a_mac_unlike_the_one_detected_was_set_by_hand() {
        let detected = parse(
            r#"
            mac = "84:90:0A:63:A9:94"
            detected_macs = ["84:90:0a:63:a9:94", "64:e4:a5:6c:ab:b4"]
            "#,
        );
        assert!(!detected.mac_set_by_hand());
        let typed = parse(
            r#"
            mac = "64:e4:a5:6c:ab:b4"
            detected_macs = ["84:90:0a:63:a9:94", "64:e4:a5:6c:ab:b4"]
            "#,
        );
        assert!(typed.mac_set_by_hand());
        assert!(parse(r#"mac = "64:e4:a5:6c:ab:b4""#).mac_set_by_hand());
        assert!(!parse(r#"detected_macs = ["64:e4:a5:6c:ab:b4"]"#).mac_set_by_hand());
    }

    #[test]
    fn refuses_a_room_no_device_could_have() {
        assert!(parse(r#"room = "Living Room""#).validate().is_err());
    }
}

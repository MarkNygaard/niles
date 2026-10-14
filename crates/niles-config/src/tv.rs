//! TV configuration section.

use crate::error::{Error, Result};
use niles_core::RoomName;
use serde::{Deserialize, Serialize};

/// The webOS websocket transport. Legacy plaintext is opt-in and should
/// only be used on a trusted local network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Tls,
    LegacyPlaintext,
}

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
/// transport = "tls"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvConfig {
    /// Off keeps the address and the pairing without using them.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    /// TLS remains the default; legacy_plaintext is unencrypted.
    #[serde(default)]
    pub transport: Transport,
    /// For waking it. Read from the TV at pairing, so rarely typed.
    #[serde(default)]
    pub mac: String,
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
            transport: Transport::Tls,
            mac: String::new(),
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

    pub fn validate(&self) -> Result<()> {
        if !self.room.is_empty() {
            RoomName::parse(&self.room)
                .map_err(|e| invalid(format!("tv.room = {:?}: {e}", self.room)))?;
        }
        let mac = self.mac.trim();
        if !mac.is_empty() {
            let parts: Vec<&str> = mac.split([':', '-']).collect();
            let well_formed = parts.len() == 6
                && parts
                    .iter()
                    .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok());
            if !well_formed {
                return Err(invalid(format!("tv.mac = {mac:?} is not a MAC address")));
            }
        }
        Ok(())
    }
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
    fn refuses_a_room_no_device_could_have() {
        assert!(parse(r#"room = "Living Room""#).validate().is_err());
    }

    #[test]
    fn transport_defaults_to_tls_and_legacy_round_trips() {
        assert_eq!(parse(r#"host = "192.168.1.2""#).transport, Transport::Tls);
        let legacy: TvConfig = parse("transport = \"legacy_plaintext\"");
        assert_eq!(legacy.transport, Transport::LegacyPlaintext);
        let encoded = toml::to_string(&TransportConfig {
            transport: legacy.transport,
        })
        .unwrap();
        assert_eq!(
            toml::from_str::<TransportConfig>(&encoded)
                .unwrap()
                .transport,
            Transport::LegacyPlaintext
        );
    }

    #[test]
    fn unknown_transport_is_rejected() {
        assert!(toml::from_str::<TvConfig>(r#"transport = "plain""#).is_err());
    }

    #[derive(Serialize, Deserialize)]
    struct TransportConfig {
        transport: Transport,
    }
}

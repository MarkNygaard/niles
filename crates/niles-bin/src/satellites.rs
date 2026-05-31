//! Satellite registry for mapping peer IPs to canonical room names.

use niles_core::RoomName;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

/// Runtime registry built from the `[satellites]` config section.
/// Maps IP address → canonical room name so `handle_transcript`
/// can thread origin context into the Tier 1 system prompt.
#[derive(Debug, Default, Clone)]
pub struct SatelliteRegistry {
    pub by_ip: HashMap<IpAddr, RoomName>,
}

impl SatelliteRegistry {
    /// Build a registry from a validated `SatellitesConfig`.
    ///
    /// Skips (with a warning) any entries whose IP or room fails to
    /// parse — `Config::validate` should already have caught these,
    /// but we are defensive on the hot path.
    pub fn from_config(cfg: &niles_config::SatellitesConfig) -> Self {
        let mut by_ip = HashMap::new();
        for (name, sat) in &cfg.satellites {
            let Ok(ip) = sat.ip.parse::<IpAddr>() else {
                tracing::warn!(
                    "skipping satellite {name}: ip={:?} is not a valid IP",
                    sat.ip
                );
                continue;
            };
            let Ok(room) = RoomName::parse(&sat.room) else {
                tracing::warn!(
                    "skipping satellite {name}: room={:?} is not a valid room name",
                    sat.room
                );
                continue;
            };
            if by_ip.insert(ip, room).is_some() {
                tracing::warn!(
                    "duplicate IP {ip} for satellite {name}, overwriting previous entry"
                );
            }
        }
        Self { by_ip }
    }

    /// Look up the room for a given peer address.
    ///
    /// Keys on `peer.ip()` only — the source port varies per Wyoming
    /// connection, so we ignore it.
    pub fn room_for(&self, peer: SocketAddr) -> Option<&RoomName> {
        self.by_ip.get(&peer.ip())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_config::SatellitesConfig;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn from_config_populates_by_ip() {
        let mut satellites = HashMap::new();
        satellites.insert(
            "living_room_sat".to_string(),
            niles_config::SatelliteConfig {
                ip: "192.168.1.10".to_string(),
                room: "living_room".to_string(),
            },
        );
        satellites.insert(
            "kitchen_sat".to_string(),
            niles_config::SatelliteConfig {
                ip: "192.168.1.20".to_string(),
                room: "kitchen".to_string(),
            },
        );
        let cfg = SatellitesConfig { satellites };
        let reg = SatelliteRegistry::from_config(&cfg);
        assert_eq!(reg.by_ip.len(), 2);
        let peer_lr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 12345);
        assert_eq!(reg.room_for(peer_lr).unwrap().as_str(), "living_room");
        let peer_k = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 54321);
        assert_eq!(reg.room_for(peer_k).unwrap().as_str(), "kitchen");
    }

    #[test]
    fn default_registry_returns_none_for_every_peer() {
        let reg = SatelliteRegistry::default();
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 12345);
        assert!(reg.room_for(peer).is_none());
    }

    #[test]
    fn room_for_matches_on_ip_ignores_port() {
        let mut satellites = HashMap::new();
        satellites.insert(
            "sat".to_string(),
            niles_config::SatelliteConfig {
                ip: "10.0.0.5".to_string(),
                room: "bedroom".to_string(),
            },
        );
        let cfg = SatellitesConfig { satellites };
        let reg = SatelliteRegistry::from_config(&cfg);
        let peer1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)), 12345);
        let peer2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)), 9999);
        let peer3 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6)), 12345);
        assert_eq!(reg.room_for(peer1).unwrap().as_str(), "bedroom");
        assert_eq!(reg.room_for(peer2).unwrap().as_str(), "bedroom");
        assert!(reg.room_for(peer3).is_none());
    }

    #[test]
    fn skips_malformed_entries_but_keeps_valid() {
        let mut satellites = HashMap::new();
        satellites.insert(
            "bad".to_string(),
            niles_config::SatelliteConfig {
                ip: "not-an-ip".to_string(),
                room: "living_room".to_string(),
            },
        );
        satellites.insert(
            "good".to_string(),
            niles_config::SatelliteConfig {
                ip: "192.168.1.1".to_string(),
                room: "living_room".to_string(),
            },
        );
        let cfg = SatellitesConfig { satellites };
        let reg = SatelliteRegistry::from_config(&cfg);
        assert_eq!(reg.by_ip.len(), 1);
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 11111);
        assert_eq!(reg.room_for(peer).unwrap().as_str(), "living_room");
    }
}

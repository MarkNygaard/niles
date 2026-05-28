//! Speaker registry built from config.

use niles_core::RoomName;
use niles_speakers::SonosClient;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct SpeakerRegistry {
    pub by_room: HashMap<RoomName, Arc<SonosClient>>,
}

impl SpeakerRegistry {
    pub fn from_config(cfg: &niles_config::SpeakersConfig) -> Self {
        let mut by_room = HashMap::new();
        for (room, sp) in &cfg.rooms {
            match RoomName::parse(room) {
                Ok(rn) => {
                    by_room.insert(rn, Arc::new(SonosClient::new(sp.ip.clone())));
                }
                Err(e) => {
                    tracing::warn!("[speakers] skipping invalid room name {room:?}: {e}");
                }
            }
        }
        Self { by_room }
    }

    pub fn get(&self, room: &RoomName) -> Option<Arc<SonosClient>> {
        self.by_room.get(room).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_yields_empty_registry() {
        let cfg = niles_config::SpeakersConfig::default();
        let reg = SpeakerRegistry::from_config(&cfg);
        assert!(reg.by_room.is_empty());
    }

    #[test]
    fn valid_room_inserts_client() {
        let mut cfg = niles_config::SpeakersConfig::default();
        cfg.rooms.insert(
            "living_room".into(),
            niles_config::SpeakerConfig {
                ip: "192.168.69.174".into(),
                kind: "sonos".into(),
            },
        );
        let reg = SpeakerRegistry::from_config(&cfg);
        assert_eq!(reg.by_room.len(), 1);
        let key = RoomName::parse("living_room").unwrap();
        assert!(reg.by_room.contains_key(&key));
    }

    #[test]
    fn unparseable_room_name_is_skipped() {
        let mut cfg = niles_config::SpeakersConfig::default();
        // "living room" contains a space, which RoomName::parse rejects.
        cfg.rooms.insert(
            "living room".into(),
            niles_config::SpeakerConfig {
                ip: "192.168.69.174".into(),
                kind: "sonos".into(),
            },
        );
        let reg = SpeakerRegistry::from_config(&cfg);
        assert!(reg.by_room.is_empty());
    }
}

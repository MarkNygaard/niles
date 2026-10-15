//! Room presentation configuration section.

use crate::error::{Error, Result};
use niles_core::RoomName;
use serde::Deserialize;
use std::collections::HashSet;

/// `[rooms]` section of the config file.
///
/// Optional, and about presentation rather than behaviour: a house has
/// an order its rooms are thought about in — the one you walk through,
/// or the one you use most — and it is never the alphabet. The
/// dashboard used to sort by name, which put the room somebody opens
/// twenty times a day wherever its initial happened to land.
///
/// Rooms the order does not name are not hidden. A light paired into a
/// new room has to appear somewhere, and appearing after the arranged
/// ones is the answer that does not make it look lost. Only `hidden`
/// hides.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RoomsConfig {
    /// The rooms that have been arranged, first on the page first.
    #[serde(default)]
    pub order: Vec<String>,
    /// The rooms left off the dashboard's grid. Only the grid: voice,
    /// rules and the API still reach them, and the house's own counts
    /// still include them. A room nothing is in any more is harmless
    /// here, since a room can come back when something is paired into
    /// it again.
    #[serde(default)]
    pub hidden: Vec<String>,
}

impl RoomsConfig {
    pub fn validate(&self) -> Result<()> {
        for (key, list) in [("order", &self.order), ("hidden", &self.hidden)] {
            let mut seen = HashSet::new();
            for room in list {
                RoomName::parse(room).map_err(|e| invalid(format!("{key}: room {room:?}: {e}")))?;
                // A room named twice has two places to be, and whichever
                // one wins, the list on screen is not the list that was
                // saved. Better rejected than quietly deduplicated.
                if !seen.insert(room) {
                    return Err(invalid(format!("{key}: room {room:?} is listed twice")));
                }
            }
        }
        Ok(())
    }
}

fn invalid(reason: String) -> Error {
    Error::InvalidSection {
        section: "rooms",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooms(order: &[&str]) -> RoomsConfig {
        RoomsConfig {
            order: order.iter().map(|s| s.to_string()).collect(),
            ..RoomsConfig::default()
        }
    }

    fn hiding(hidden: &[&str]) -> RoomsConfig {
        RoomsConfig {
            hidden: hidden.iter().map(|s| s.to_string()).collect(),
            ..RoomsConfig::default()
        }
    }

    #[test]
    fn an_empty_order_is_fine() {
        // The default, and what a house that has never arranged its
        // rooms has. It means "no opinion", not "no rooms".
        assert!(RoomsConfig::default().validate().is_ok());
    }

    #[test]
    fn accepts_rooms_by_their_canonical_name() {
        assert!(
            rooms(&["living_room", "kitchen", "bedroom"])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn rejects_a_name_no_device_could_have() {
        // The names here have to be the ones device ids carry, or the
        // entry orders a room that cannot exist.
        let err = rooms(&["Living Room"]).validate().unwrap_err();
        assert!(err.to_string().contains("rooms"), "{err}");
    }

    #[test]
    fn rejects_a_room_listed_twice() {
        let err = rooms(&["kitchen", "office", "kitchen"])
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }

    #[test]
    fn hides_rooms_by_their_canonical_name() {
        assert!(hiding(&["garage", "loft"]).validate().is_ok());
    }

    #[test]
    fn rejects_hiding_a_name_no_device_could_have() {
        let err = hiding(&["The Garage"]).validate().unwrap_err();
        assert!(err.to_string().contains("hidden"), "{err}");
    }

    #[test]
    fn rejects_a_room_hidden_twice() {
        let err = hiding(&["garage", "garage"]).validate().unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }

    #[test]
    fn a_room_can_be_both_arranged_and_hidden() {
        // Hiding keeps its place, so showing it again puts it back
        // where it was rather than at the end.
        let config = RoomsConfig {
            order: vec!["kitchen".into(), "garage".into()],
            hidden: vec!["garage".into()],
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn reads_hidden_from_the_file() {
        let config: RoomsConfig = toml::from_str(
            r#"
            order = ["kitchen"]
            hidden = ["garage", "no_such_room"]
            "#,
        )
        .unwrap();
        assert_eq!(config.hidden, ["garage", "no_such_room"]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn hidden_defaults_to_nothing() {
        let config: RoomsConfig = toml::from_str(r#"order = ["kitchen"]"#).unwrap();
        assert!(config.hidden.is_empty());
    }
}

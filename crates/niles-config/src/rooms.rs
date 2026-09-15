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
/// ones is the answer that does not make it look lost.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RoomsConfig {
    /// The rooms that have been arranged, first on the page first.
    #[serde(default)]
    pub order: Vec<String>,
}

impl RoomsConfig {
    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for room in &self.order {
            RoomName::parse(room).map_err(|e| invalid(format!("room {room:?}: {e}")))?;
            // A room named twice has two places to be, and whichever
            // one wins, the list on screen is not the list that was
            // saved. Better rejected than quietly deduplicated.
            if !seen.insert(room) {
                return Err(invalid(format!("room {room:?} is listed twice")));
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
}

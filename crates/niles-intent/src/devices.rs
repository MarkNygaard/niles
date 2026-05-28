//! Device index — maps spoken device names to [`DeviceId`]s.
//!
//! Pure runtime-built type: caller feeds it [`DeviceId`]s, and it
//! indexes by the spoken form of the device name (underscores
//! replaced with spaces). No bus coupling inside this module.

use niles_core::DeviceId;
use std::collections::HashMap;

/// Index of devices by their spoken (space-separated) name.
///
/// Multiple devices may share the same spoken name across different
/// rooms; [`matches`](Self::matches) returns all candidates so the
/// caller can disambiguate.
#[derive(Debug, Clone, Default)]
pub struct DeviceIndex {
    by_name: HashMap<String, Vec<DeviceId>>,
}

impl DeviceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a device into the index.
    pub fn insert(&mut self, id: DeviceId) {
        let key = canonicalize_phrase(&spoken_device_name(id.name().as_str()));
        self.by_name.entry(key).or_default().push(id);
    }

    /// Remove a device from the index.
    ///
    /// If this empties the bucket, the key is removed entirely.
    pub fn remove(&mut self, id: &DeviceId) {
        let key = canonicalize_phrase(&spoken_device_name(id.name().as_str()));
        if let Some(bucket) = self.by_name.get_mut(&key) {
            bucket.retain(|x| x != id);
            if bucket.is_empty() {
                self.by_name.remove(&key);
            }
        }
    }

    /// Look up candidates by spoken phrase.
    ///
    /// The phrase is canonicalised (lowercased, collapsed whitespace)
    /// before lookup. Returns an empty slice when nothing matches.
    pub fn matches(&self, phrase: &str) -> &[DeviceId] {
        let key = canonicalize_phrase(phrase);
        self.by_name
            .get(&key)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// True when the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Total number of indexed devices (counting duplicates across
    /// keys, which is the normal case since each device only lives
    /// under its own name).
    pub fn len(&self) -> usize {
        self.by_name.values().map(|v| v.len()).sum()
    }
}

/// Convert a canonical device name (`floor_lamp`) to its spoken form
/// (`floor lamp`).
fn spoken_device_name(raw: &str) -> String {
    raw.replace('_', " ").to_lowercase()
}

/// Lowercase and collapse internal whitespace.
fn canonicalize_phrase(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{DeviceName, RoomName};

    fn make_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn insert_and_matches_round_trip() {
        let mut idx = DeviceIndex::new();
        let id = make_id("living_room", "floor_lamp");
        idx.insert(id.clone());
        assert_eq!(idx.matches("floor lamp"), &[id]);
    }

    #[test]
    fn spoken_name_derivation() {
        let mut idx = DeviceIndex::new();
        let id = make_id("living_room", "floor_lamp");
        idx.insert(id.clone());
        // lookup by the spoken form succeeds
        assert_eq!(idx.matches("floor lamp"), &[id]);
    }

    #[test]
    fn multi_room_match_returns_both() {
        let mut idx = DeviceIndex::new();
        let living = make_id("living_room", "floor_lamp");
        let bedroom = make_id("bedroom", "floor_lamp");
        idx.insert(living.clone());
        idx.insert(bedroom.clone());
        let result = idx.matches("floor lamp");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], living);
        assert_eq!(result[1], bedroom);
    }

    #[test]
    fn remove_shrinks_bucket() {
        let mut idx = DeviceIndex::new();
        let id = make_id("living_room", "floor_lamp");
        idx.insert(id.clone());
        assert!(!idx.is_empty());
        idx.remove(&id);
        assert!(idx.is_empty());
        assert!(idx.matches("floor lamp").is_empty());
    }

    #[test]
    fn remove_one_of_two_keeps_other() {
        let mut idx = DeviceIndex::new();
        let living = make_id("living_room", "floor_lamp");
        let bedroom = make_id("bedroom", "floor_lamp");
        idx.insert(living.clone());
        idx.insert(bedroom.clone());
        idx.remove(&living);
        assert_eq!(idx.matches("floor lamp"), &[bedroom]);
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn canonicalize_phrase_collapses_whitespace() {
        assert_eq!(canonicalize_phrase("  Floor   LAMP  "), "floor lamp");
    }

    #[test]
    fn unknown_phrase_returns_empty() {
        let idx = DeviceIndex::new();
        assert!(idx.matches("ceiling light").is_empty());
    }

    #[test]
    fn len_counts_all_devices() {
        let mut idx = DeviceIndex::new();
        idx.insert(make_id("living_room", "floor_lamp"));
        idx.insert(make_id("bedroom", "floor_lamp"));
        idx.insert(make_id("kitchen", "ceiling_light"));
        assert_eq!(idx.len(), 3);
    }
}

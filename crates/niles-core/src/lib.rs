//! niles-core — event bus, device registry, shared types.
//!
//! This crate has no business logic. It provides the type system other
//! crates compose against: device identifiers, the runtime registry,
//! and the internal event bus.

pub mod device;
pub mod error;
pub mod event;
pub mod registry;

pub use device::{
    Device, DeviceClass, DeviceId, DeviceName, DeviceState, LightCapabilities, RoomName,
};
pub use error::{Error, Result};
pub use event::{Event, EventBus, PresenceState};
pub use registry::DeviceRegistry;

/// What a spoken name is stored as.
///
/// `"Movie Night"` and `"movie night"` are the same scene, and the
/// same timer. Lowercase, and single underscores between words, so a
/// name survives being said twice with different spacing.
///
/// Here rather than beside the first thing that needed it, because
/// three now do — the timer store, the scene store, and the router
/// deciding whether a spoken word is the name of a scene. A copy per
/// crate is a copy to get wrong.
pub fn canonicalize_name(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(test)]
mod canonicalize_tests {
    use super::canonicalize_name;

    #[test]
    fn spacing_and_case_do_not_make_a_second_scene() {
        assert_eq!(canonicalize_name("  Movie   Night "), "movie_night");
        assert_eq!(canonicalize_name("movie night"), "movie_night");
    }
}

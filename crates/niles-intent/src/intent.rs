//! Structured intents produced by the Tier 0 router.

use std::time::Duration;

/// A structured representation of what the user asked for.
///
/// Text fields preserve the user's phrasing (`"living room"`, not
/// `"living_room"`) — the resolution layer maps to canonical
/// identifiers when dispatching.
///
/// `#[non_exhaustive]` so new variants don't break downstream matches.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Intent {
    /// "turn off the kitchen light(s)" / "bedroom lights on"
    LightSet {
        room: String,
        on: bool,
    },

    /// "dim the kitchen lights to 30%" / "set the bedroom light to 50 percent"
    ///
    /// `percent` is `0..=100`. The router rejects values outside that
    /// range so the caller never has to clamp.
    LightDim {
        room: String,
        percent: u8,
    },

    /// "back to normal" / "normal lights" (whole-home) or
    /// "back to normal in <room>" / "<room> back to normal" (room-scoped).
    ClearManualMode {
        /// `None` = clear flags for *every* device in the registry.
        /// `Some(s)` = clear flags only for devices in that room. The
        /// raw room string is passed through; canonicalization happens
        /// at dispatch time (same pattern as `LightSet { room }`).
        room: Option<String>,
    },

    /// "save this as <name>" / "save <room> as <name>" / "save <name>"
    SceneSave {
        /// The scene name as the user said it. Canonicalization
        /// (lowercase, whitespace -> underscore) happens at dispatch
        /// time in `SceneStore::save`.
        name: String,
        /// `None` = whole-home snapshot.
        /// `Some(s)` = restrict to that room. Same raw-string contract
        /// as `LightSet { room }` — canonicalize via
        /// `intent_room_to_canonical` at dispatch time.
        room: Option<String>,
    },

    /// "apply <name>" / "<name> scene" / "scene <name>" — matched
    /// only when the transcript uses an explicit scene-recall phrasing.
    /// Bare `<name>` is intentionally rejected at the router level.
    SceneApply {
        name: String,
    },

    /// "set a timer for 5 minutes" / "5 minute timer" / "10 minute timer called pasta"
    TimerSet {
        duration: Duration,
        name: Option<String>,
    },

    /// Acknowledgments used to stop an in-progress alarm.
    Stop,
    Cancel,
}

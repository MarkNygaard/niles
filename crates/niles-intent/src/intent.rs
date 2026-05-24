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

    /// "set a timer for 5 minutes" / "5 minute timer" / "10 minute timer called pasta"
    TimerSet {
        duration: Duration,
        name: Option<String>,
    },

    /// Acknowledgments used to stop an in-progress alarm.
    Stop,
    Cancel,
}

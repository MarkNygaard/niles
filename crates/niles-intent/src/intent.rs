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

    /// "set a timer for 5 minutes" / "5 minute timer" / "10 minute timer called pasta"
    TimerSet {
        duration: Duration,
        name: Option<String>,
    },

    /// Acknowledgments used to stop an in-progress alarm.
    Stop,
    Cancel,
}

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

    /// "turn on the floor lamp" / "turn off the ceiling light"
    ///
    /// Device-name-targeted variant for single-device control.
    DeviceSet {
        device_id: niles_core::DeviceId,
        on: bool,
    },

    /// "dim the floor lamp to 30%" / "set the ceiling light to 50 percent"
    ///
    /// Device-name-targeted dim, same percent range as `LightDim`.
    DeviceDim {
        device_id: niles_core::DeviceId,
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

    /// "list my scenes" / "what scenes do I have" / "list scenes" /
    /// "show me my scenes"
    SceneList,

    /// "delete the kitchen evening scene" / "remove kitchen evening
    /// scene" / "delete scene kitchen evening" / "remove scene kitchen
    /// evening". `name` is the raw user-said name; `SceneStore::delete`
    /// canonicalizes.
    SceneDelete {
        name: String,
    },

    /// "pause the living room" / "pause music in the kitchen"
    MediaPause {
        room: String,
    },

    /// "play the kitchen" / "resume music in the living room"
    MediaPlay {
        room: String,
    },

    /// "set the kitchen volume to 30%" / "living room volume to 40 percent"
    MediaVolumeSet {
        room: String,
        percent: u8,
    },

    /// "volume up in the kitchen" / "kitchen volume down"
    MediaVolumeStep {
        room: String,
        delta: i16,
    },

    /// "set a timer for 5 minutes" / "5 minute timer" / "10 minute timer called pasta"
    TimerSet {
        duration: Duration,
        name: Option<String>,
    },

    /// "cancel the pasta timer" / "stop the pasta timer" — named
    /// cancellation. `name` is the raw user-said name; `TimerStore`
    /// canonicalizes (trim + ASCII lowercase + whitespace -> `_`).
    TimerCancel {
        name: String,
    },

    /// "list my timers" / "what timers do I have"
    TimerList,

    /// Acknowledgments used to stop an in-progress alarm.
    Stop,
    Cancel,
}

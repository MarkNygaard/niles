//! niles-scheduler — time-driven behaviors.
//!
//! This crate hosts the lighting curve, the morning routine, and the
//! timer scheduler. The current module set is:
//!
//! - [`time`] — the `MinuteOfDay` type used throughout.
//! - [`curve`] — the daily brightness curve.
//! - [`manual_mode`] — the manual-mode tracker for per-light override.
//! - [`morning`] — the morning auto-turn-on routine + claim tracker.

pub mod curve;
pub mod error;
pub mod manual_mode;
pub mod morning;
pub mod scenes;
pub mod sink;
pub mod time;

pub use curve::{CurveConfig, Phase, brightness_at, color_temp_at, phase_at};
pub use error::{Error, Result};
pub use manual_mode::ManualModeTracker;
pub use morning::{
    MorningClaimTracker, MorningRoutineConfig, routine_brightness_at, should_fire_today,
};
pub use scenes::{SceneEntry, SceneStore};
pub use sink::{BRIGHTNESS_DEBOUNCE, build_curve_target};
pub use time::MinuteOfDay;

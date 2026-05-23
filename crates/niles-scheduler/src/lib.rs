//! niles-scheduler — time-driven behaviors.
//!
//! This crate hosts the lighting curve, the morning routine, and the
//! timer scheduler. The current module set is:
//!
//! - [`time`] — the `MinuteOfDay` type used throughout.
//! - [`curve`] — the daily brightness curve.
//!
//! The morning routine, timer subsystem, and color-temperature curve
//! land in follow-up PRs.

pub mod curve;
pub mod error;
pub mod sink;
pub mod time;

pub use curve::{CurveConfig, Phase, brightness_at, color_temp_at, phase_at};
pub use error::{Error, Result};
pub use sink::{BRIGHTNESS_DEBOUNCE, KELVIN_DEBOUNCE_K, build_curve_target};
pub use time::MinuteOfDay;

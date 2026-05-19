//! The daily lighting curve.
//!
//! Per the architecture spec, the curve is continuous: the value at
//! every minute is well-defined and never jumps. The morning ramp goes
//! from `night_floor_brightness` to `daytime_brightness`, not from 0%.
//! The "0% → 100%" ramp described in the spec is the *morning routine*'s
//! concern, not the curve's, and lands in a separate module.

use crate::error::{Error, Result};
use crate::time::MinuteOfDay;

/// Inputs to the brightness curve.
///
/// All times are wall-clock for the local day. Day-pattern resolution
/// (which day's sunset time applies, whether the routine fires today)
/// happens above this layer — by the time the config reaches the curve,
/// it is the resolved values for the current day.
#[derive(Debug, Clone)]
pub struct CurveConfig {
    pub morning_start: MinuteOfDay,
    pub morning_end: MinuteOfDay,
    pub sunset_start: MinuteOfDay,
    pub sunset_end: MinuteOfDay,
    /// Brightness during the night phase (`0..=100`).
    pub night_floor_brightness: u8,
    /// Brightness during the daytime plateau (`0..=100`).
    pub daytime_brightness: u8,
    /// Color-temperature anchors throughout the day, in chronological order.
    ///
    /// Between adjacent anchors, color temperature is linearly
    /// interpolated. Before the first anchor or after the last,
    /// the nearest anchor's value is returned (no wrap-around).
    /// Must be non-empty; values are validated to be in the
    /// `1000..=10000` Kelvin range.
    pub color_temp_anchors: Vec<(MinuteOfDay, u16)>,
}

impl CurveConfig {
    /// Architecture-spec defaults: morning 05:45–06:30, sunset 21:30–23:00,
    /// night floor 15%, daytime 100%, color temp 2000K (night) → 2700K
    /// (morning end) → 4500K (midday peak) → 3500K (afternoon) → 2700K
    /// (sunset start) → 2000K (sunset end). The bookend anchors at 00:00
    /// and 23:59 pin the flat night sections so out-of-range lookups
    /// return the night-floor color rather than the first/last transition.
    pub fn default_weekday() -> Self {
        Self {
            morning_start: MinuteOfDay::new(5, 45).expect("05:45 is valid"),
            morning_end: MinuteOfDay::new(6, 30).expect("06:30 is valid"),
            sunset_start: MinuteOfDay::new(21, 30).expect("21:30 is valid"),
            sunset_end: MinuteOfDay::new(23, 0).expect("23:00 is valid"),
            night_floor_brightness: 15,
            daytime_brightness: 100,
            color_temp_anchors: vec![
                (MinuteOfDay::new(0, 0).expect("00:00 is valid"), 2000),
                (MinuteOfDay::new(5, 45).expect("05:45 is valid"), 2000),
                (MinuteOfDay::new(6, 30).expect("06:30 is valid"), 2700),
                (MinuteOfDay::new(12, 0).expect("12:00 is valid"), 4500),
                (MinuteOfDay::new(17, 0).expect("17:00 is valid"), 3500),
                (MinuteOfDay::new(21, 30).expect("21:30 is valid"), 2700),
                (MinuteOfDay::new(23, 0).expect("23:00 is valid"), 2000),
                (MinuteOfDay::new(23, 59).expect("23:59 is valid"), 2000),
            ],
        }
    }

    /// Validate field ordering and brightness ranges. Cross-midnight
    /// ramps (e.g. weekend sunset extending past 23:59) are not yet
    /// supported and produce an error.
    pub fn validate(&self) -> Result<()> {
        if self.morning_start >= self.morning_end {
            return Err(Error::InvalidConfig {
                reason: format!(
                    "morning_start {} must be < morning_end {}",
                    self.morning_start, self.morning_end
                ),
            });
        }
        if self.morning_end > self.sunset_start {
            return Err(Error::InvalidConfig {
                reason: format!(
                    "morning_end {} must be <= sunset_start {}",
                    self.morning_end, self.sunset_start
                ),
            });
        }
        if self.sunset_start >= self.sunset_end {
            return Err(Error::InvalidConfig {
                reason: format!(
                    "sunset_start {} must be < sunset_end {}",
                    self.sunset_start, self.sunset_end
                ),
            });
        }
        if self.night_floor_brightness > 100 {
            return Err(Error::InvalidConfig {
                reason: format!(
                    "night_floor_brightness {} > 100",
                    self.night_floor_brightness
                ),
            });
        }
        if self.daytime_brightness > 100 {
            return Err(Error::InvalidConfig {
                reason: format!("daytime_brightness {} > 100", self.daytime_brightness),
            });
        }
        if self.color_temp_anchors.is_empty() {
            return Err(Error::InvalidConfig {
                reason: "color_temp_anchors must not be empty".into(),
            });
        }
        for window in self.color_temp_anchors.windows(2) {
            if window[0].0 >= window[1].0 {
                return Err(Error::InvalidConfig {
                    reason: format!(
                        "color_temp_anchors must be sorted ascending by time: {} >= {}",
                        window[0].0, window[1].0
                    ),
                });
            }
        }
        for (time, kelvin) in &self.color_temp_anchors {
            if !(1000..=10000).contains(kelvin) {
                return Err(Error::InvalidConfig {
                    reason: format!(
                        "color_temp anchor at {time}: {kelvin}K outside valid 1000..=10000 range"
                    ),
                });
            }
        }
        Ok(())
    }
}

/// Which phase of the curve a given time falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before `morning_start` or at/after `sunset_end`.
    Night,
    /// `morning_start..morning_end`. Brightness ramps `night_floor` → `daytime`.
    MorningRamp,
    /// `morning_end..sunset_start`. Brightness sits at the daytime plateau.
    Day,
    /// `sunset_start..sunset_end`. Brightness ramps `daytime` → `night_floor`.
    SunsetRamp,
}

/// Which phase the curve is in at `time`.
///
/// Caller must pass a config accepted by [`CurveConfig::validate`].
/// With an unvalidated config (e.g. inverted ramp ordering) the
/// returned phase is unspecified but the call never panics.
pub fn phase_at(config: &CurveConfig, time: MinuteOfDay) -> Phase {
    let t = time.total_minutes();
    let morning_start = config.morning_start.total_minutes();
    let morning_end = config.morning_end.total_minutes();
    let sunset_start = config.sunset_start.total_minutes();
    let sunset_end = config.sunset_end.total_minutes();

    if t < morning_start || t >= sunset_end {
        Phase::Night
    } else if t < morning_end {
        Phase::MorningRamp
    } else if t < sunset_start {
        Phase::Day
    } else {
        Phase::SunsetRamp
    }
}

/// Brightness at `time` (0..=100).
///
/// Continuous within the rounding of integer-minute discretization:
/// adjacent minutes differ by at most ~2 brightness units, and the
/// curve never jumps from the daytime plateau straight to the night
/// floor (the spec bug clarified in PR #4).
///
/// Caller must pass a config accepted by [`CurveConfig::validate`];
/// behavior on an unvalidated config is unspecified.
pub fn brightness_at(config: &CurveConfig, time: MinuteOfDay) -> u8 {
    let t = time.total_minutes();
    match phase_at(config, time) {
        Phase::Night => config.night_floor_brightness,
        Phase::Day => config.daytime_brightness,
        Phase::MorningRamp => {
            let start = config.morning_start.total_minutes();
            let end = config.morning_end.total_minutes();
            lerp_brightness(
                config.night_floor_brightness,
                config.daytime_brightness,
                t - start,
                end - start,
            )
        }
        Phase::SunsetRamp => {
            let start = config.sunset_start.total_minutes();
            let end = config.sunset_end.total_minutes();
            lerp_brightness(
                config.daytime_brightness,
                config.night_floor_brightness,
                t - start,
                end - start,
            )
        }
    }
}

/// Color temperature at `time`, in Kelvin.
///
/// Linearly interpolated between adjacent anchors in
/// `config.color_temp_anchors`. Before the first anchor or after the
/// last, the nearest anchor's value is returned (no wrap-around).
///
/// Caller must pass a config accepted by [`CurveConfig::validate`].
/// On an unvalidated config (empty anchors, out-of-order) behavior
/// is unspecified but the call never panics.
pub fn color_temp_at(config: &CurveConfig, time: MinuteOfDay) -> u16 {
    let anchors = &config.color_temp_anchors;
    debug_assert!(
        !anchors.is_empty(),
        "color_temp_at called with empty color_temp_anchors — caller must validate the config first"
    );
    let Some(first) = anchors.first() else {
        return 2000; // safe fallback for unvalidated empty config in release
    };
    let last = anchors
        .last()
        .expect("anchors non-empty per first.is_some()");

    let t = time.total_minutes();
    if t <= first.0.total_minutes() {
        return first.1;
    }
    if t >= last.0.total_minutes() {
        return last.1;
    }
    for window in anchors.windows(2) {
        let a = window[0];
        let b = window[1];
        let a_t = a.0.total_minutes();
        let b_t = b.0.total_minutes();
        if t >= a_t && t < b_t {
            return lerp_kelvin(a.1, b.1, t - a_t, b_t - a_t);
        }
    }
    last.1 // unreachable for sorted anchors
}

/// Linear interpolation between two brightness values.
/// Math is done in `i32` so reversed ramps (sunset goes high → low)
/// work and no `u8` overflow is possible.
fn lerp_brightness(from: u8, to: u8, numerator: u16, denominator: u16) -> u8 {
    if denominator == 0 {
        return from;
    }
    let from = i32::from(from);
    let to = i32::from(to);
    let num = i32::from(numerator);
    let den = i32::from(denominator);
    let result = from + (to - from) * num / den;
    result.clamp(0, 100) as u8
}

/// Linear interpolation between two Kelvin values.
///
/// `from` and `to` are guaranteed to be in `1000..=10000` by
/// [`CurveConfig::validate`], so the result is too — no clamp needed.
fn lerp_kelvin(from: u16, to: u16, numerator: u16, denominator: u16) -> u16 {
    if denominator == 0 {
        return from;
    }
    let from = i32::from(from);
    let to = i32::from(to);
    let num = i32::from(numerator);
    let den = i32::from(denominator);
    (from + (to - from) * num / den) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CurveConfig {
        CurveConfig::default_weekday()
    }

    fn t(hour: u8, minute: u8) -> MinuteOfDay {
        MinuteOfDay::new(hour, minute).unwrap()
    }

    // ---- phase_at -------------------------------------------------

    #[test]
    fn phase_at_night() {
        assert_eq!(phase_at(&cfg(), t(0, 0)), Phase::Night);
        assert_eq!(phase_at(&cfg(), t(3, 0)), Phase::Night);
        assert_eq!(phase_at(&cfg(), t(5, 44)), Phase::Night);
        assert_eq!(phase_at(&cfg(), t(23, 0)), Phase::Night); // sunset_end exclusive
        assert_eq!(phase_at(&cfg(), t(23, 59)), Phase::Night);
    }

    #[test]
    fn phase_at_morning_ramp() {
        assert_eq!(phase_at(&cfg(), t(5, 45)), Phase::MorningRamp);
        assert_eq!(phase_at(&cfg(), t(6, 0)), Phase::MorningRamp);
        assert_eq!(phase_at(&cfg(), t(6, 29)), Phase::MorningRamp);
    }

    #[test]
    fn phase_at_day() {
        assert_eq!(phase_at(&cfg(), t(6, 30)), Phase::Day);
        assert_eq!(phase_at(&cfg(), t(12, 0)), Phase::Day);
        assert_eq!(phase_at(&cfg(), t(21, 29)), Phase::Day);
    }

    #[test]
    fn phase_at_sunset_ramp() {
        assert_eq!(phase_at(&cfg(), t(21, 30)), Phase::SunsetRamp);
        assert_eq!(phase_at(&cfg(), t(22, 15)), Phase::SunsetRamp);
        assert_eq!(phase_at(&cfg(), t(22, 59)), Phase::SunsetRamp);
    }

    // ---- brightness_at --------------------------------------------

    #[test]
    fn brightness_night() {
        assert_eq!(brightness_at(&cfg(), t(3, 0)), 15);
        assert_eq!(brightness_at(&cfg(), t(0, 0)), 15);
        assert_eq!(brightness_at(&cfg(), t(23, 30)), 15);
    }

    #[test]
    fn brightness_morning_start_is_night_floor() {
        // The architecture's continuity invariant: no jump at 05:45.
        assert_eq!(brightness_at(&cfg(), t(5, 45)), 15);
    }

    #[test]
    fn brightness_morning_end_is_daytime() {
        assert_eq!(brightness_at(&cfg(), t(6, 30)), 100);
    }

    #[test]
    fn brightness_worked_example_06_10() {
        // From ARCHITECTURE.md's manual-turn-on table:
        // 25 min into a 45 min ramp, 15 → 100 → 15 + (25/45) * 85 ≈ 62.
        let b = brightness_at(&cfg(), t(6, 10));
        assert!((61..=63).contains(&b), "got {b}");
    }

    #[test]
    fn brightness_day_plateau() {
        assert_eq!(brightness_at(&cfg(), t(12, 0)), 100);
        assert_eq!(brightness_at(&cfg(), t(15, 0)), 100);
        assert_eq!(brightness_at(&cfg(), t(21, 29)), 100);
    }

    #[test]
    fn brightness_sunset_start_is_daytime() {
        assert_eq!(brightness_at(&cfg(), t(21, 30)), 100);
    }

    #[test]
    fn brightness_sunset_end_is_night_floor() {
        // sunset_end is exclusive — 23:00 is in Night phase.
        assert_eq!(brightness_at(&cfg(), t(23, 0)), 15);
    }

    #[test]
    fn brightness_sunset_ramp_midpoint() {
        // 22:15 = 45 min into 90 min ramp; 100 - (45/90) * 85 = 57.5.
        let b = brightness_at(&cfg(), t(22, 15));
        assert!((57..=58).contains(&b), "got {b}");
    }

    // ---- continuity across phase boundaries -----------------------

    // Curve is sampled at integer minutes, so adjacent minutes near a
    // ramp endpoint differ by at most ~2 brightness units. The spec
    // bug fixed in PR #4 was a 15→0 jump at 05:45; this tolerance is
    // small enough that any reintroduction would fail the assertion.
    const MAX_ADJACENT_MINUTE_DELTA: u8 = 2;

    fn assert_continuous_across(cfg: &CurveConfig, before: MinuteOfDay, after: MinuteOfDay) {
        let b = brightness_at(cfg, before);
        let a = brightness_at(cfg, after);
        let delta = b.abs_diff(a);
        assert!(
            delta <= MAX_ADJACENT_MINUTE_DELTA,
            "discontinuity across {before}→{after}: {b}→{a} (delta {delta})"
        );
    }

    #[test]
    fn brightness_continuous_at_morning_start() {
        // The original spec-bug boundary: Night → MorningRamp.
        assert_continuous_across(&cfg(), t(5, 44), t(5, 45));
    }

    #[test]
    fn brightness_continuous_at_morning_end() {
        // MorningRamp → Day.
        assert_continuous_across(&cfg(), t(6, 29), t(6, 30));
    }

    #[test]
    fn brightness_continuous_at_sunset_start() {
        // Day → SunsetRamp.
        assert_continuous_across(&cfg(), t(21, 29), t(21, 30));
    }

    #[test]
    fn brightness_continuous_at_sunset_end() {
        // SunsetRamp → Night.
        assert_continuous_across(&cfg(), t(22, 59), t(23, 0));
    }

    // ---- monotonic sweeps -----------------------------------------

    fn minute_offset(base: MinuteOfDay, offset: u16) -> MinuteOfDay {
        let total = base.total_minutes() + offset;
        MinuteOfDay::new((total / 60) as u8, (total % 60) as u8).unwrap()
    }

    #[test]
    fn brightness_monotonic_through_morning_ramp() {
        let cfg = cfg();
        let span = cfg.morning_end.total_minutes() - cfg.morning_start.total_minutes();
        let mut last = brightness_at(&cfg, cfg.morning_start);
        for offset in 1..span {
            let time = minute_offset(cfg.morning_start, offset);
            let b = brightness_at(&cfg, time);
            assert!(b >= last, "non-monotonic: {time} = {b} < previous {last}");
            last = b;
        }
    }

    #[test]
    fn brightness_monotonic_through_sunset_ramp() {
        let cfg = cfg();
        let span = cfg.sunset_end.total_minutes() - cfg.sunset_start.total_minutes();
        let mut last = brightness_at(&cfg, cfg.sunset_start);
        for offset in 1..span {
            let time = minute_offset(cfg.sunset_start, offset);
            let b = brightness_at(&cfg, time);
            assert!(b <= last, "non-monotonic: {time} = {b} > previous {last}");
            last = b;
        }
    }

    // ---- validate -------------------------------------------------

    #[test]
    fn validate_default_ok() {
        cfg().validate().unwrap();
    }

    #[test]
    fn validate_rejects_inverted_morning() {
        let mut c = cfg();
        std::mem::swap(&mut c.morning_start, &mut c.morning_end);
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_morning_after_sunset_start() {
        let c = CurveConfig {
            morning_start: t(20, 0),
            morning_end: t(22, 0),
            sunset_start: t(21, 0),
            sunset_end: t(23, 0),
            night_floor_brightness: 15,
            daytime_brightness: 100,
            color_temp_anchors: cfg().color_temp_anchors,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_accepts_zero_length_day_plateau() {
        // morning_end == sunset_start is degenerate but valid: the
        // morning ramp hands directly to the sunset ramp with no Day
        // plateau in between. Useful for short winter days.
        let c = CurveConfig {
            morning_start: t(5, 45),
            morning_end: t(12, 0),
            sunset_start: t(12, 0),
            sunset_end: t(23, 0),
            night_floor_brightness: 15,
            daytime_brightness: 100,
            color_temp_anchors: cfg().color_temp_anchors,
        };
        c.validate().unwrap();
        // At the seam, phase is SunsetRamp (Day is the open interval).
        assert_eq!(phase_at(&c, t(12, 0)), Phase::SunsetRamp);
        assert_eq!(brightness_at(&c, t(12, 0)), 100);
    }

    #[test]
    fn validate_rejects_brightness_over_100() {
        let mut c = cfg();
        c.night_floor_brightness = 101;
        assert!(c.validate().is_err());

        let mut c = cfg();
        c.daytime_brightness = 200;
        assert!(c.validate().is_err());
    }

    // ---- color_temp_at --------------------------------------------

    #[test]
    fn color_temp_night_anchor() {
        // Anchored at 2000K at 00:00 and 05:45; flat between.
        assert_eq!(color_temp_at(&cfg(), t(0, 0)), 2000);
        assert_eq!(color_temp_at(&cfg(), t(3, 0)), 2000);
        assert_eq!(color_temp_at(&cfg(), t(5, 45)), 2000);
    }

    #[test]
    fn color_temp_morning_ramp_interpolates() {
        // 05:45 = 2000K, 06:30 = 2700K. 06:07 is 22 min in of 45 min ramp.
        // 2000 + (22/45) * 700 ≈ 2342.
        let k = color_temp_at(&cfg(), t(6, 7));
        assert!((2330..=2360).contains(&k), "got {k}K");
    }

    #[test]
    fn color_temp_morning_end_is_anchor_value() {
        assert_eq!(color_temp_at(&cfg(), t(6, 30)), 2700);
    }

    #[test]
    fn color_temp_midday_peak() {
        // Anchor at 12:00 = 4500K.
        assert_eq!(color_temp_at(&cfg(), t(12, 0)), 4500);
    }

    #[test]
    fn color_temp_warms_into_afternoon() {
        // 12:00 = 4500K, 17:00 = 3500K. Midpoint 14:30 ≈ 4000K.
        let k = color_temp_at(&cfg(), t(14, 30));
        assert!((3990..=4010).contains(&k), "got {k}K");
    }

    #[test]
    fn color_temp_sunset_anchors() {
        assert_eq!(color_temp_at(&cfg(), t(21, 30)), 2700);
        assert_eq!(color_temp_at(&cfg(), t(23, 0)), 2000);
        assert_eq!(color_temp_at(&cfg(), t(23, 59)), 2000);
    }

    #[test]
    fn color_temp_clamps_to_first_anchor_before_start() {
        // The default anchors start at 00:00, but if a custom config had
        // a later first anchor, times before it should return that anchor.
        let mut c = cfg();
        c.color_temp_anchors = vec![(t(6, 0), 3000), (t(18, 0), 4000), (t(22, 0), 2200)];
        c.validate().unwrap();
        assert_eq!(color_temp_at(&c, t(0, 0)), 3000);
        assert_eq!(color_temp_at(&c, t(5, 0)), 3000);
        // And after the last anchor:
        assert_eq!(color_temp_at(&c, t(23, 0)), 2200);
    }

    #[test]
    fn color_temp_continuous_at_morning_end_boundary() {
        // The 06:30 anchor pins both sides; adjacent minutes must match
        // within rounding.
        let before = color_temp_at(&cfg(), t(6, 29));
        let after = color_temp_at(&cfg(), t(6, 30));
        assert!(before.abs_diff(after) <= 20, "got {before}K -> {after}K");
    }

    #[test]
    fn validate_rejects_empty_color_temp_anchors() {
        let mut c = cfg();
        c.color_temp_anchors = vec![];
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_unsorted_color_temp_anchors() {
        let mut c = cfg();
        c.color_temp_anchors = vec![(t(12, 0), 4500), (t(6, 0), 2700)];
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_color_temp_outside_kelvin_range() {
        let mut c = cfg();
        c.color_temp_anchors = vec![(t(12, 0), 500)]; // too cold
        assert!(c.validate().is_err());

        let mut c = cfg();
        c.color_temp_anchors = vec![(t(12, 0), 12000)]; // too hot
        assert!(c.validate().is_err());
    }
}

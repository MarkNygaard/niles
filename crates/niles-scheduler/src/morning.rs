//! Morning routine — auto-turn-on at sunrise with a 0% → 100% brightness ramp.
//!
//! The routine lives in `niles serve`. It claims target devices at
//! `morning_start` on configured fire-days, drives them through a
//! linear brightness ramp during the curve window, and releases the
//! claim at `morning_end` so the ambient curve takes over.

use chrono::{Datelike, NaiveDate, Weekday};
use niles_core::DeviceId;
use std::collections::HashSet;
use std::sync::RwLock;

use crate::time::MinuteOfDay;

/// Runtime configuration for the morning routine.
#[derive(Debug, Clone)]
pub struct MorningRoutineConfig {
    /// Weekdays on which the routine should fire (e.g. Mon–Fri).
    pub fire_days: Vec<Weekday>,
    /// Devices to claim and ramp.
    pub target_devices: Vec<DeviceId>,
    /// Dates on which the routine should be skipped even if the
    /// weekday matches.
    pub skip_overrides: Vec<NaiveDate>,
}

/// Whether the routine should fire on the given calendar day.
///
/// Skip overrides take precedence: if `today` appears in
/// `cfg.skip_overrides`, the routine is disabled for that day
/// regardless of the weekday.
pub fn should_fire_today(cfg: &MorningRoutineConfig, today: NaiveDate) -> bool {
    if cfg.skip_overrides.contains(&today) {
        return false;
    }
    cfg.fire_days.contains(&today.weekday())
}

/// Compute the target brightness for the routine at a given time.
///
/// - `time < morning_start` → `None` (routine not yet active).
/// - `time == morning_start` → `Some(0)`.
/// - `morning_start < time < morning_end` → linear interpolation
///   from 0 to 100.
/// - `time >= morning_end` → `Some(100)`.
///
/// # Precondition
///
/// `morning_start < morning_end`. Callers must validate ordering
/// before calling this function (the upstream `CurveConfig::validate`
/// already enforces this).
pub fn routine_brightness_at(
    time: MinuteOfDay,
    morning_start: MinuteOfDay,
    morning_end: MinuteOfDay,
) -> Option<u8> {
    let t = time.total_minutes();
    let start = morning_start.total_minutes();
    let end = morning_end.total_minutes();

    debug_assert!(start < end, "morning_start must be < morning_end");

    if t < start {
        return None;
    }
    if t >= end {
        return Some(100);
    }

    let span = (end - start) as u32;
    let elapsed = (t - start) as u32;
    let pct = (elapsed * 100 / span) as u8;
    Some(pct)
}

/// Tracks which devices are currently claimed by the morning routine.
///
/// Shape is byte-for-byte parallel to [`ManualModeTracker`](crate::manual_mode::ManualModeTracker)
/// minus the `last_on` map and `observe()` method — claim release is
/// purely event-driven (off-state observed by the `niles-bin` task).
pub struct MorningClaimTracker {
    claimed: RwLock<HashSet<DeviceId>>,
}

impl MorningClaimTracker {
    pub fn new() -> Self {
        Self {
            claimed: RwLock::new(HashSet::new()),
        }
    }

    /// Claim `id` for the routine — the curve driver should skip it.
    pub fn claim(&self, id: &DeviceId) {
        self.claimed_write().insert(id.clone());
    }

    /// Release `id` from the routine, returning it to curve control.
    pub fn release(&self, id: &DeviceId) {
        self.claimed_write().remove(id);
    }

    /// True if `id` is currently claimed.
    pub fn is_claimed(&self, id: &DeviceId) -> bool {
        self.claimed_read().contains(id)
    }

    /// Drop all tracker state for `id`. Use when a device is removed
    /// from the registry so its entry doesn't linger forever.
    pub fn forget(&self, id: &DeviceId) {
        self.claimed_write().remove(id);
    }

    // ---- lock helpers -------------------------------------------------

    fn claimed_write(&self) -> std::sync::RwLockWriteGuard<'_, HashSet<DeviceId>> {
        self.claimed.write().unwrap_or_else(|e| e.into_inner())
    }

    fn claimed_read(&self) -> std::sync::RwLockReadGuard<'_, HashSet<DeviceId>> {
        self.claimed.read().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for MorningClaimTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:test/{name}")).unwrap()
    }

    // ------------------------------------------------------------------
    // should_fire_today
    // ------------------------------------------------------------------

    #[test]
    fn should_fire_today_on_fire_day() {
        let cfg = MorningRoutineConfig {
            fire_days: vec![Weekday::Mon],
            target_devices: vec![],
            skip_overrides: vec![],
        };
        assert!(should_fire_today(
            &cfg,
            NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()
        ));
    }

    #[test]
    fn should_fire_today_off_day() {
        let cfg = MorningRoutineConfig {
            fire_days: vec![Weekday::Mon],
            target_devices: vec![],
            skip_overrides: vec![],
        };
        assert!(!should_fire_today(
            &cfg,
            NaiveDate::from_ymd_opt(2026, 5, 26).unwrap()
        ));
    }

    #[test]
    fn should_fire_today_skip_override_wins() {
        let skip = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap(); // a Monday
        let cfg = MorningRoutineConfig {
            fire_days: vec![Weekday::Mon],
            target_devices: vec![],
            skip_overrides: vec![skip],
        };
        assert!(!should_fire_today(&cfg, skip));
    }

    // ------------------------------------------------------------------
    // routine_brightness_at
    // ------------------------------------------------------------------

    #[test]
    fn routine_brightness_at_start_is_zero() {
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();
        assert_eq!(routine_brightness_at(start, start, end), Some(0));
    }

    #[test]
    fn routine_brightness_at_end_is_100() {
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();
        assert_eq!(routine_brightness_at(end, start, end), Some(100));
    }

    #[test]
    fn routine_brightness_at_past_end_is_100() {
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();
        let past = MinuteOfDay::new(7, 0).unwrap();
        assert_eq!(routine_brightness_at(past, start, end), Some(100));
    }

    #[test]
    fn routine_brightness_at_before_window_is_none() {
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();
        let before = MinuteOfDay::new(5, 0).unwrap();
        assert_eq!(routine_brightness_at(before, start, end), None);
    }

    #[test]
    fn routine_brightness_at_midpoint() {
        let start = MinuteOfDay::new(5, 0).unwrap();
        let end = MinuteOfDay::new(6, 0).unwrap();
        let mid = MinuteOfDay::new(5, 30).unwrap();
        let b = routine_brightness_at(mid, start, end).unwrap();
        // 30/60 * 100 = 50, but integer rounding can give ±1.
        assert!((48..=52).contains(&b), "expected ~50, got {b}");
    }

    #[test]
    fn routine_brightness_continuity() {
        // Adjacent minutes should differ by at most 3. The exact bound
        // depends on the window span and integer rounding; 3 covers all
        // realistic morning windows (e.g. 45 min → 100/45 ≈ 2.2 per min).
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();

        for m in start.total_minutes()..end.total_minutes() {
            let a = MinuteOfDay::new((m / 60) as u8, (m % 60) as u8).unwrap();
            let b = MinuteOfDay::new(((m + 1) / 60) as u8, ((m + 1) % 60) as u8).unwrap();
            let ba = routine_brightness_at(a, start, end).unwrap();
            let bb = routine_brightness_at(b, start, end).unwrap();
            assert!(
                ba.abs_diff(bb) <= 3,
                "jump between {a} and {b}: {ba} -> {bb}"
            );
        }
    }

    #[test]
    fn routine_brightness_monotonically_non_decreasing() {
        let start = MinuteOfDay::new(5, 45).unwrap();
        let end = MinuteOfDay::new(6, 30).unwrap();
        let mut prev = 0u8;
        for m in start.total_minutes()..=end.total_minutes() {
            let t = MinuteOfDay::new((m / 60) as u8, (m % 60) as u8).unwrap();
            let b = routine_brightness_at(t, start, end).unwrap_or(0);
            assert!(b >= prev, "brightness decreased at {t}: {prev} -> {b}");
            prev = b;
        }
    }

    // ------------------------------------------------------------------
    // MorningClaimTracker
    // ------------------------------------------------------------------

    #[test]
    fn claim_release_roundtrip() {
        let t = MorningClaimTracker::new();
        let id = dev("light_a");
        assert!(!t.is_claimed(&id));
        t.claim(&id);
        assert!(t.is_claimed(&id));
        t.release(&id);
        assert!(!t.is_claimed(&id));
    }

    #[test]
    fn is_claimed_false_for_unknown_device() {
        let t = MorningClaimTracker::new();
        assert!(!t.is_claimed(&dev("never_touched")));
    }

    #[test]
    fn arc_clones_share_writes() {
        use std::sync::Arc;
        let t = Arc::new(MorningClaimTracker::new());
        let t2 = t.clone();
        let id = dev("light_g");
        t.claim(&id);
        assert!(t2.is_claimed(&id));
    }

    #[test]
    fn forget_clears_state() {
        let t = MorningClaimTracker::new();
        let id = dev("light_h");
        t.claim(&id);
        assert!(t.is_claimed(&id));
        t.forget(&id);
        assert!(!t.is_claimed(&id));
    }

    #[test]
    fn forget_is_idempotent_on_unknown_device() {
        let t = MorningClaimTracker::new();
        t.forget(&dev("never_touched"));
    }

    #[test]
    fn default_is_new() {
        let t: MorningClaimTracker = Default::default();
        assert!(!t.is_claimed(&dev("x")));
    }
}

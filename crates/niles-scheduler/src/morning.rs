//! Morning routine — auto-turn-on at sunrise with a 0% → 100% brightness ramp.
//!
//! The routine lives in `niles serve`. It claims target devices at
//! `morning_start` on configured fire-days, drives them through a
//! linear brightness ramp during the curve window, and releases the
//! claim at `morning_end` so the ambient curve takes over.

use chrono::{Datelike, NaiveDate, Weekday};
use niles_core::DeviceId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::persistence::{atomic_write_json, read_json_or_empty};
use crate::time::MinuteOfDay;

/// Runtime configuration for the morning routine.
#[derive(Debug, Clone)]
pub struct MorningRoutineConfig {
    /// Weekdays on which the routine should fire (e.g. Mon–Fri).
    pub fire_days: Vec<Weekday>,
    /// Devices to claim and ramp. Empty = all curve-managed lights.
    pub target_devices: Vec<DeviceId>,
    /// Lights to exclude from the routine, applied after target
    /// resolution. Most useful with an empty `target_devices` — i.e.
    /// "all lights except these".
    pub exclude_devices: Vec<DeviceId>,
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

/// Why the routine is leaving a device alone at the start minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Skipped {
    /// Named in the routine, but nothing is reporting it.
    Unknown,
    /// An ambient light, or not a light at all.
    NotCurveDriven,
    AlreadyOn,
    AlreadyClaimed,
}

impl Skipped {
    /// What to put in the log. A wake-up that does not happen is
    /// noticed hours later in a dark room, so the reason has to be
    /// readable then, not inferred from the code now.
    pub fn reason(self) -> &'static str {
        match self {
            Skipped::Unknown => "no such device — named in the routine, but nothing reports it",
            Skipped::NotCurveDriven => "not curve-driven (an ambient light, or not a light)",
            Skipped::AlreadyOn => "already on",
            Skipped::AlreadyClaimed => "already claimed",
        }
    }
}

/// Whether to switch a device on at the start of the morning window.
///
/// `None` means go. Split out of the tick so the rule can be tested:
/// it was wrong once, and the way it was wrong cost two mornings and
/// left nothing behind to read.
///
/// **Manual mode is deliberately not an input.** The flag protects a
/// level somebody chose, and a light that is off has no level to
/// protect — `AlreadyOn` turns back anything lit, so everything
/// reaching the rest of this is off or has never reported. The flag
/// only clears on an off→on transition, so consulting it here meant
/// one evening of dimming a light by hand switched the wake-up off for
/// every morning afterwards, until somebody happened to turn that
/// light on and clear it. The ramp still respects the flag, which is
/// where it belongs: mid-window, that is a light somebody is adjusting
/// now.
pub fn kick_on_skip(
    present: bool,
    curve_driven: bool,
    on: Option<bool>,
    claimed: bool,
) -> Option<Skipped> {
    if !present {
        return Some(Skipped::Unknown);
    }
    if !curve_driven {
        return Some(Skipped::NotCurveDriven);
    }
    if on == Some(true) {
        return Some(Skipped::AlreadyOn);
    }
    if claimed {
        return Some(Skipped::AlreadyClaimed);
    }
    None
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
#[derive(Serialize, Deserialize, Default)]
struct PersistedClaims {
    device_ids: Vec<String>,
}

pub struct MorningClaimTracker {
    claimed: RwLock<HashSet<DeviceId>>,
    persistence_path: Option<PathBuf>,
}

impl MorningClaimTracker {
    pub fn new() -> Self {
        Self {
            claimed: RwLock::new(HashSet::new()),
            persistence_path: None,
        }
    }

    pub fn with_persistence(mut self, path: PathBuf) -> Self {
        self.persistence_path = Some(path);
        self
    }

    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let persisted: PersistedClaims = read_json_or_empty(path, "morning_claims")?;
        let mut claimed = HashSet::new();
        for raw in persisted.device_ids {
            match DeviceId::parse(&raw) {
                Ok(id) => {
                    claimed.insert(id);
                }
                Err(_) => {
                    tracing::warn!(
                        "persistence: dropping morning claim with malformed device_id '{}'",
                        raw
                    );
                }
            }
        }
        Ok(Self {
            claimed: RwLock::new(claimed),
            persistence_path: None,
        })
    }

    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let inner = self.claimed_read();
        self.save_locked(&inner, path)
    }

    fn save_locked(&self, inner: &HashSet<DeviceId>, path: &Path) -> std::io::Result<()> {
        let mut device_ids: Vec<String> = inner.iter().map(|id| id.to_string()).collect();
        device_ids.sort_unstable();
        atomic_write_json(path, &PersistedClaims { device_ids })
    }

    fn maybe_save(&self, inner: &HashSet<DeviceId>) {
        if let Some(path) = self.persistence_path.as_deref()
            && let Err(e) = self.save_locked(inner, path)
        {
            tracing::warn!("persistence: morning_claims save failed: {e}");
        }
    }

    pub fn claimed_count(&self) -> usize {
        self.claimed_read().len()
    }

    /// Claim `id` for the routine — the curve driver should skip it.
    pub fn claim(&self, id: &DeviceId) {
        let mut inner = self.claimed_write();
        inner.insert(id.clone());
        self.maybe_save(&inner);
    }

    /// Release `id` from the routine, returning it to curve control.
    pub fn release(&self, id: &DeviceId) {
        let mut inner = self.claimed_write();
        inner.remove(id);
        self.maybe_save(&inner);
    }

    /// True if `id` is currently claimed.
    pub fn is_claimed(&self, id: &DeviceId) -> bool {
        self.claimed_read().contains(id)
    }

    /// Drop all tracker state for `id`. Use when a device is removed
    /// from the registry so its entry doesn't linger forever.
    pub fn forget(&self, id: &DeviceId) {
        let mut inner = self.claimed_write();
        inner.remove(id);
        self.maybe_save(&inner);
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
    fn a_light_set_by_hand_last_night_still_wakes_you_up() {
        // The regression this rule was extracted for. Manual mode
        // clears only on an off→on transition, so one evening of
        // dimming a light by hand used to switch the wake-up off for
        // every morning afterwards. The flag is not an input here at
        // all: an off light has no level to protect.
        assert_eq!(kick_on_skip(true, true, Some(false), false), None);
        assert_eq!(kick_on_skip(true, true, None, false), None);
    }

    #[test]
    fn a_lit_light_is_left_where_it_is() {
        assert_eq!(
            kick_on_skip(true, true, Some(true), false),
            Some(Skipped::AlreadyOn)
        );
    }

    #[test]
    fn a_device_nothing_reports_is_named_rather_than_ignored() {
        // It is a name in the config that matches nothing — worth
        // saying out loud, because the alternative is a routine that
        // looks configured and does nothing.
        assert_eq!(
            kick_on_skip(false, false, None, false),
            Some(Skipped::Unknown)
        );
    }

    #[test]
    fn an_ambient_light_sits_the_routine_out() {
        assert_eq!(
            kick_on_skip(true, false, Some(false), false),
            Some(Skipped::NotCurveDriven)
        );
    }

    #[test]
    fn a_claim_is_not_made_twice() {
        assert_eq!(
            kick_on_skip(true, true, Some(false), true),
            Some(Skipped::AlreadyClaimed)
        );
    }

    #[test]
    fn every_reason_says_something_a_person_can_read() {
        for reason in [
            Skipped::Unknown,
            Skipped::NotCurveDriven,
            Skipped::AlreadyOn,
            Skipped::AlreadyClaimed,
        ] {
            assert!(!reason.reason().is_empty());
        }
    }

    #[test]
    fn should_fire_today_on_fire_day() {
        let cfg = MorningRoutineConfig {
            fire_days: vec![Weekday::Mon],
            target_devices: vec![],
            exclude_devices: vec![],
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
            exclude_devices: vec![],
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
            exclude_devices: vec![],
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

    // ------------------------------------------------------------------
    // Persistence tests
    // ------------------------------------------------------------------

    #[test]
    fn persists_and_reloads_claim_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("morning_claims.json");
        let tracker = MorningClaimTracker::new().with_persistence(path.clone());
        let id1 = dev("light_a");
        let id2 = dev("light_b");
        tracker.claim(&id1);
        tracker.claim(&id2);

        let reloaded = MorningClaimTracker::load_from_file(&path)
            .unwrap()
            .with_persistence(path);
        assert!(reloaded.is_claimed(&id1));
        assert!(reloaded.is_claimed(&id2));
        assert_eq!(reloaded.claimed_count(), 2);
    }

    #[test]
    fn release_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("morning_claims.json");
        let tracker = MorningClaimTracker::new().with_persistence(path.clone());
        let id = dev("light_a");
        tracker.claim(&id);

        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["device_ids"].as_array().unwrap().len(), 1);

        tracker.release(&id);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["device_ids"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn load_from_missing_file_yields_empty_tracker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let tracker = MorningClaimTracker::load_from_file(&path).unwrap();
        assert_eq!(tracker.claimed_count(), 0);
    }

    #[test]
    fn load_from_corrupt_file_yields_empty_tracker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("morning_claims.json");
        std::fs::write(&path, b"not json").unwrap();
        let tracker = MorningClaimTracker::load_from_file(&path).unwrap();
        assert_eq!(tracker.claimed_count(), 0);
    }
}

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
}

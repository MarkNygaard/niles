//! Per-light manual-mode flag store backing the curve dispatcher's skip decision.
//!
//! Off→on transitions auto-clear the flag per `ARCHITECTURE.md`.

use niles_core::{DeviceId, DeviceState, RoomName};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

/// Tracks which lights are in "manual mode" (exempt from curve adjustments).
///
/// A device enters manual mode when a voice `LightDim` command targets it.
/// It exits manual mode on the next off→on transition observed via
/// [`ManualModeTracker::observe`], or when explicitly [`clear`](Self::clear)ed.
///
/// The tracker is pure sync logic: it uses `std::sync::RwLock` so it can be
/// called from both sync and async contexts without adding a tokio dependency
/// to `niles-scheduler`.
pub struct ManualModeTracker {
    flagged: RwLock<HashSet<DeviceId>>,
    last_on: RwLock<HashMap<DeviceId, bool>>,
}

impl ManualModeTracker {
    pub fn new() -> Self {
        Self {
            flagged: RwLock::new(HashSet::new()),
            last_on: RwLock::new(HashMap::new()),
        }
    }

    /// Mark `id` as manually controlled — the curve driver should skip it.
    pub fn flag(&self, id: &DeviceId) {
        self.flagged_write().insert(id.clone());
    }

    /// Remove the manual-mode flag for `id`, returning it to curve control.
    pub fn clear(&self, id: &DeviceId) {
        self.flagged_write().remove(id);
    }

    /// Clear the manual-mode flag for *every* tracked device.
    /// Returns the number of devices that had been flagged.
    ///
    /// Leaves `last_on` untouched so the off→on auto-clear contract
    /// continues to work for any flag added later.
    pub fn clear_all(&self) -> usize {
        let mut flagged = self.flagged_write();
        let n = flagged.len();
        flagged.clear();
        n
    }

    /// Clear the manual-mode flag for every device whose ID's room
    /// segment matches `room`. Returns the number of devices cleared.
    ///
    /// The caller passes a canonicalized `RoomName` so the tracker
    /// doesn't have to know about transcript normalization.
    pub fn clear_room(&self, room: &RoomName) -> usize {
        let mut flagged = self.flagged_write();
        let before = flagged.len();
        flagged.retain(|id| id.room() != room);
        before - flagged.len()
    }

    /// True if `id` is currently flagged.
    pub fn is_flagged(&self, id: &DeviceId) -> bool {
        self.flagged_read().contains(id)
    }

    /// Drop *all* tracker state for `id` — both the manual-mode flag and
    /// the last-seen on-state. Use when a device is removed from the
    /// registry so its entries don't linger forever.
    pub fn forget(&self, id: &DeviceId) {
        self.flagged_write().remove(id);
        self.last_on_write().remove(id);
    }

    /// Observe a state update and auto-clear the flag on off→on transitions.
    ///
    /// If `state.on` is `None` the observation is ignored — we only track
    /// the boolean on-state.  When `state.on` is `Some(new_on)`:
    ///
    /// - The previous on-state is looked up in `last_on`.
    /// - If the previous value was `Some(false)` and `new_on` is `true`,
    ///   the manual-mode flag is cleared (off→on auto-clear).
    /// - In all `Some(new_on)` cases the last-seen value is updated.
    ///
    /// Lock ordering: `last_on` is always acquired before `flagged` so that
    /// any future code that needs both locks avoids deadlock.
    pub fn observe(&self, id: &DeviceId, state: &DeviceState) {
        let new_on = match state.on {
            Some(v) => v,
            None => return,
        };

        let prev = {
            let mut last_on = self.last_on_write();
            let prev = last_on.get(id).copied();
            last_on.insert(id.clone(), new_on);
            prev
        };

        if prev == Some(false) && new_on {
            self.clear(id);
        }
    }

    // ---- lock helpers -----------------------------------------------------
    //
    // `std::sync::RwLock` poisons on a panic in any thread holding the
    // write lock. Our operations are trivial (HashSet/HashMap inserts
    // and removes) and don't panic in practice, but defensively
    // recovering from poison via `into_inner()` means a panic in one
    // path can't cascade-kill the whole tracker. The recovered state
    // is consistent because each individual lock guards a single
    // collection that's mutated atomically inside the critical
    // section.

    fn flagged_write(&self) -> std::sync::RwLockWriteGuard<'_, HashSet<DeviceId>> {
        self.flagged.write().unwrap_or_else(|e| e.into_inner())
    }

    fn flagged_read(&self) -> std::sync::RwLockReadGuard<'_, HashSet<DeviceId>> {
        self.flagged.read().unwrap_or_else(|e| e.into_inner())
    }

    fn last_on_write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<DeviceId, bool>> {
        self.last_on.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for ManualModeTracker {
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

    fn on_state(on: bool) -> DeviceState {
        DeviceState {
            on: Some(on),
            ..Default::default()
        }
    }

    fn dev_in(room: &str, name: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{room}/{name}")).unwrap()
    }

    #[test]
    fn flag_then_is_flagged_roundtrip() {
        let t = ManualModeTracker::new();
        let id = dev("light_a");
        assert!(!t.is_flagged(&id));
        t.flag(&id);
        assert!(t.is_flagged(&id));
    }

    #[test]
    fn clear_after_flag() {
        let t = ManualModeTracker::new();
        let id = dev("light_b");
        t.flag(&id);
        assert!(t.is_flagged(&id));
        t.clear(&id);
        assert!(!t.is_flagged(&id));
    }

    #[test]
    fn is_flagged_false_for_unknown_device() {
        let t = ManualModeTracker::new();
        assert!(!t.is_flagged(&dev("never_touched")));
    }

    #[test]
    fn observe_with_no_prior_state_does_not_clear() {
        let t = ManualModeTracker::new();
        let id = dev("light_c");
        t.flag(&id);
        t.observe(&id, &on_state(true));
        assert!(t.is_flagged(&id));
    }

    #[test]
    fn observe_off_to_on_clears_flag() {
        let t = ManualModeTracker::new();
        let id = dev("light_d");
        t.flag(&id);
        t.observe(&id, &on_state(false));
        assert!(t.is_flagged(&id));
        t.observe(&id, &on_state(true));
        assert!(!t.is_flagged(&id));
    }

    #[test]
    fn observe_on_to_on_preserves_flag() {
        let t = ManualModeTracker::new();
        let id = dev("light_e");
        t.flag(&id);
        t.observe(&id, &on_state(true));
        assert!(t.is_flagged(&id));
        t.observe(&id, &on_state(true));
        assert!(t.is_flagged(&id));
    }

    #[test]
    fn observe_on_to_off_preserves_flag() {
        let t = ManualModeTracker::new();
        let id = dev("light_f");
        t.flag(&id);
        t.observe(&id, &on_state(true));
        assert!(t.is_flagged(&id));
        t.observe(&id, &on_state(false));
        assert!(t.is_flagged(&id));
    }

    #[test]
    fn arc_clones_share_writes() {
        use std::sync::Arc;
        let t = Arc::new(ManualModeTracker::new());
        let t2 = t.clone();
        let id = dev("light_g");
        t.flag(&id);
        assert!(t2.is_flagged(&id));
    }

    #[test]
    fn forget_clears_flag_and_last_on() {
        let t = ManualModeTracker::new();
        let id = dev("light_h");
        t.flag(&id);
        t.observe(&id, &on_state(true));
        t.forget(&id);
        assert!(!t.is_flagged(&id));
        // After forget, the next observe with no prior should NOT
        // trigger a spurious clear — verify the `last_on` row is
        // also gone by re-checking the off->on transition contract:
        // observe(true) alone (no prior false) must not clear a
        // freshly re-set flag.
        t.flag(&id);
        t.observe(&id, &on_state(true));
        assert!(
            t.is_flagged(&id),
            "forget should have wiped last_on too, so observe(true) has no prior=false to trigger clear"
        );
    }

    #[test]
    fn forget_is_idempotent_on_unknown_device() {
        let t = ManualModeTracker::new();
        // No prior flag, no prior observe — must not panic.
        t.forget(&dev("never_touched"));
    }

    #[test]
    fn clear_all_empties_and_returns_count() {
        let t = ManualModeTracker::new();
        let a = dev("a");
        let b = dev("b");
        let c = dev("c");
        t.flag(&a);
        t.flag(&b);
        t.flag(&c);
        assert_eq!(t.clear_all(), 3);
        assert!(!t.is_flagged(&a));
        assert!(!t.is_flagged(&b));
        assert!(!t.is_flagged(&c));
    }

    #[test]
    fn clear_all_returns_zero_when_empty() {
        let t = ManualModeTracker::new();
        assert_eq!(t.clear_all(), 0);
    }

    #[test]
    fn clear_room_only_clears_matching_room() {
        let t = ManualModeTracker::new();
        let kitchen_a = dev_in("kitchen", "a");
        let kitchen_b = dev_in("kitchen", "b");
        let bedroom_a = dev_in("bedroom", "a");
        t.flag(&kitchen_a);
        t.flag(&kitchen_b);
        t.flag(&bedroom_a);
        let kitchen = RoomName::parse("kitchen").unwrap();
        assert_eq!(t.clear_room(&kitchen), 2);
        assert!(!t.is_flagged(&kitchen_a));
        assert!(!t.is_flagged(&kitchen_b));
        assert!(t.is_flagged(&bedroom_a));
    }

    #[test]
    fn clear_room_returns_zero_when_no_match() {
        let t = ManualModeTracker::new();
        t.flag(&dev_in("kitchen", "a"));
        let bedroom = RoomName::parse("bedroom").unwrap();
        assert_eq!(t.clear_room(&bedroom), 0);
    }

    #[test]
    fn clear_all_leaves_last_on_intact() {
        // After clear_all, off→on auto-clear must still fire for a
        // freshly re-flagged device. That contract relies on `last_on`
        // surviving a clear_all.
        let t = ManualModeTracker::new();
        let id = dev("light");
        t.flag(&id);
        t.observe(&id, &on_state(false)); // prior=false in last_on
        assert_eq!(t.clear_all(), 1);

        t.flag(&id);
        t.observe(&id, &on_state(true)); // off->on must clear because last_on still says false
        assert!(
            !t.is_flagged(&id),
            "last_on must survive clear_all so off→on auto-clear keeps working"
        );
    }

    #[test]
    fn clear_room_leaves_last_on_intact() {
        let t = ManualModeTracker::new();
        let id = dev_in("kitchen", "ceiling");
        t.flag(&id);
        t.observe(&id, &on_state(false));
        let kitchen = RoomName::parse("kitchen").unwrap();
        assert_eq!(t.clear_room(&kitchen), 1);

        t.flag(&id);
        t.observe(&id, &on_state(true));
        assert!(
            !t.is_flagged(&id),
            "last_on must survive clear_room so off→on auto-clear keeps working"
        );
    }
}

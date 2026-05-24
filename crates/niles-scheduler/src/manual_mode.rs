//! Per-light manual-mode flag store backing the curve dispatcher's skip decision.
//!
//! Off→on transitions auto-clear the flag per `ARCHITECTURE.md`.

use niles_core::{DeviceId, DeviceState};
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
        self.flagged.write().unwrap().insert(id.clone());
    }

    /// Remove the manual-mode flag for `id`, returning it to curve control.
    pub fn clear(&self, id: &DeviceId) {
        self.flagged.write().unwrap().remove(id);
    }

    /// True if `id` is currently flagged.
    pub fn is_flagged(&self, id: &DeviceId) -> bool {
        self.flagged.read().unwrap().contains(id)
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
            let mut last_on = self.last_on.write().unwrap();
            let prev = last_on.get(id).copied();
            last_on.insert(id.clone(), new_on);
            prev
        };

        if prev == Some(false) && new_on {
            self.clear(id);
        }
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
}

//! Pure decision logic for the lighting curve dispatcher.
//!
//! Per the architecture: the curve only governs lights that are
//! already on. It does not turn lights on or off. At each tick, for
//! every currently-on light, we compute the curve's `(brightness,
//! kelvin)` at the current time and ask: is this device's current
//! state far enough from the curve to be worth sending a new set
//! command?
//!
//! "Far enough" is the debounce threshold — a tiny brightness diff
//! between adjacent minutes would otherwise spam the broker with
//! sub-perceptible updates.
//!
//! This module is the *decision*, not the dispatch. The actual MQTT
//! publish lives in the binary, which owns the publisher handle.
//! Splitting it this way keeps the math testable in isolation and
//! keeps `niles-scheduler` free of any async / I/O dependencies.

use niles_core::DeviceState;

/// Don't bother publishing if the device is already within this
/// many brightness points of the curve. Matches the curve test's
/// per-minute delta tolerance — anything tighter is meaningless to
/// the human eye and noisy on the wire.
pub(crate) const BRIGHTNESS_DEBOUNCE: u8 = 2;

/// Same idea, for color temperature. Hue/equivalent bulbs change
/// kelvin in roughly 50-K steps anyway, so this threshold avoids
/// publishing a half-step nobody can see.
pub(crate) const KELVIN_DEBOUNCE_K: u16 = 50;

/// Given a device's current state and the curve's target values,
/// build the minimal `DeviceState` to publish — or return `None`
/// if no field is meaningfully off-curve.
///
/// Important behaviors:
///
/// - Devices not declaring `brightness` (e.g. a smart plug) get
///   no brightness command, even if the curve says otherwise. The
///   same applies to color temperature.
/// - `on` is never set: the curve never turns lights on or off
///   (that's the morning routine + manual control's job).
/// - When the current value matches the curve value within the
///   debounce window, that field is omitted.
/// - When *every* field would be omitted, the function returns
///   `None`, signaling "skip publish entirely."
pub fn build_curve_target(
    current: &DeviceState,
    curve_brightness: u8,
    curve_kelvin: u16,
) -> Option<DeviceState> {
    let brightness = match current.brightness {
        Some(cur) if cur.abs_diff(curve_brightness) > BRIGHTNESS_DEBOUNCE => Some(curve_brightness),
        _ => None,
    };
    let kelvin = match current.color_temp_kelvin {
        Some(cur) if cur.abs_diff(curve_kelvin) > KELVIN_DEBOUNCE_K => Some(curve_kelvin),
        _ => None,
    };
    if brightness.is_none() && kelvin.is_none() {
        return None;
    }
    Some(DeviceState {
        brightness,
        color_temp_kelvin: kelvin,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(brightness: Option<u8>, kelvin: Option<u16>) -> DeviceState {
        DeviceState {
            on: Some(true),
            brightness,
            color_temp_kelvin: kelvin,
            ..Default::default()
        }
    }

    #[test]
    fn publishes_when_brightness_far_from_curve() {
        // Current 30, curve wants 80 — well past the 2-pt debounce.
        let target =
            build_curve_target(&state(Some(30), Some(2700)), 80, 2700).expect("should publish");
        assert_eq!(target.brightness, Some(80));
        // Kelvin already matches → omitted.
        assert_eq!(target.color_temp_kelvin, None);
    }

    #[test]
    fn publishes_when_kelvin_far_from_curve() {
        let target =
            build_curve_target(&state(Some(80), Some(2000)), 80, 4500).expect("should publish");
        assert_eq!(target.brightness, None);
        assert_eq!(target.color_temp_kelvin, Some(4500));
    }

    #[test]
    fn publishes_both_when_both_drift() {
        let target =
            build_curve_target(&state(Some(30), Some(2000)), 80, 4500).expect("should publish");
        assert_eq!(target.brightness, Some(80));
        assert_eq!(target.color_temp_kelvin, Some(4500));
    }

    #[test]
    fn skip_when_both_already_on_curve() {
        // Identical values → nothing to do.
        assert!(build_curve_target(&state(Some(80), Some(2700)), 80, 2700).is_none());
    }

    #[test]
    fn skip_when_within_debounce_window() {
        // 79 vs 80 brightness, 2680 vs 2700 K — both within debounce.
        assert!(build_curve_target(&state(Some(79), Some(2680)), 80, 2700).is_none());
        // Boundary cases: equal-to-debounce is *not* "more than", so still skip.
        assert!(build_curve_target(&state(Some(78), Some(2650)), 80, 2700).is_none());
    }

    #[test]
    fn publishes_just_past_debounce_window() {
        // 3-pt brightness diff > 2-pt debounce.
        let t = build_curve_target(&state(Some(77), Some(2700)), 80, 2700).expect("publish");
        assert_eq!(t.brightness, Some(80));
        assert_eq!(t.color_temp_kelvin, None);

        // 51-K kelvin diff > 50-K debounce.
        let t = build_curve_target(&state(Some(80), Some(2649)), 80, 2700).expect("publish");
        assert_eq!(t.brightness, None);
        assert_eq!(t.color_temp_kelvin, Some(2700));
    }

    #[test]
    fn skip_for_device_without_brightness_or_kelvin() {
        // Smart-plug-shaped state: power only, no light fields.
        assert!(build_curve_target(&state(None, None), 80, 2700).is_none());
    }

    #[test]
    fn skip_brightness_for_brightness_unaware_device() {
        // Brightness field missing → never publish brightness, even if
        // the curve says we should. Kelvin still applies if exposed.
        let target =
            build_curve_target(&state(None, Some(2000)), 80, 4500).expect("kelvin should publish");
        assert_eq!(target.brightness, None);
        assert_eq!(target.color_temp_kelvin, Some(4500));
    }

    #[test]
    fn returned_target_never_sets_on() {
        // The curve never turns lights on/off — caller already
        // filtered to currently-on devices. Make sure we don't
        // accidentally publish an `on` field that would re-trigger
        // an off→on transition (which clears manual mode, per spec).
        let target = build_curve_target(&state(Some(30), Some(2000)), 80, 4500).unwrap();
        assert_eq!(target.on, None);
    }
}

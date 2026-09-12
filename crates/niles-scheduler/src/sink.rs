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

use niles_core::{DeviceState, LightCapabilities};

/// Don't bother publishing if the device is already within this
/// many brightness points of the curve. Matches the curve test's
/// per-minute delta tolerance — anything tighter is meaningless to
/// the human eye and noisy on the wire.
pub const BRIGHTNESS_DEBOUNCE: u8 = 2;

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

/// The command to hold an ambient light at a fixed setting, or `None`
/// if it is already there.
///
/// Differs from [`build_curve_target`] in one deliberate way: an
/// unknown current colour still gets a command. The curve reads "never
/// reported" as "this device has no such channel", which is right when
/// it is only maintaining a value — but an ambient light was explicitly
/// told to be a colour, and a strip sitting in white mode reports no
/// colour at all. Waiting for it to report one first would wait
/// forever.
pub fn build_ambient_target(
    current: &DeviceState,
    capabilities: LightCapabilities,
    brightness: Option<u8>,
    kelvin: Option<u16>,
    rgb: Option<[u8; 3]>,
) -> Option<DeviceState> {
    // Each light takes the one it can actually act on. A strip with no
    // white channel cannot use a colour temperature, and a bulb with no
    // colour channel cannot use a colour — sending the wrong one is a
    // command that quietly does nothing.
    let rgb = rgb.filter(|_| capabilities.rgb);
    let kelvin = kelvin.filter(|_| capabilities.color_temp && rgb.is_none());
    let brightness = match (brightness, current.brightness) {
        (Some(want), Some(cur)) if cur.abs_diff(want) <= BRIGHTNESS_DEBOUNCE => None,
        (want, _) => want,
    };
    let kelvin = match (kelvin, current.color_temp_kelvin) {
        (Some(want), Some(cur)) if cur.abs_diff(want) <= KELVIN_DEBOUNCE_K => None,
        (want, _) => want,
    };
    let rgb = match (rgb, current.rgb) {
        (Some(want), Some(cur)) if cur == want => None,
        (want, _) => want,
    };
    if brightness.is_none() && kelvin.is_none() && rgb.is_none() {
        return None;
    }
    Some(DeviceState {
        brightness,
        color_temp_kelvin: kelvin,
        rgb,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RGB: LightCapabilities = LightCapabilities {
        color_temp: true,
        rgb: true,
    };
    const WHITE_ONLY: LightCapabilities = LightCapabilities {
        color_temp: true,
        rgb: false,
    };

    fn state(brightness: Option<u8>, kelvin: Option<u16>) -> DeviceState {
        DeviceState {
            on: Some(true),
            brightness,
            color_temp_kelvin: kelvin,
            ..Default::default()
        }
    }

    #[test]
    fn ambient_sends_a_colour_a_light_has_never_reported() {
        // A strip in white mode reports no colour at all. Waiting for
        // it to report one before sending would wait forever.
        let target = build_ambient_target(
            &state(Some(40), None),
            RGB,
            Some(40),
            None,
            Some([255, 128, 0]),
        )
        .expect("should publish");
        assert_eq!(target.rgb, Some([255, 128, 0]));
        assert_eq!(target.brightness, None, "already at 40");
    }

    #[test]
    fn ambient_is_quiet_once_the_light_is_where_it_was_told() {
        let current = DeviceState {
            on: Some(true),
            brightness: Some(40),
            rgb: Some([255, 128, 0]),
            ..Default::default()
        };
        assert!(build_ambient_target(&current, RGB, Some(40), None, Some([255, 128, 0])).is_none());
    }

    #[test]
    fn a_white_only_light_gets_the_colour_temperature_instead() {
        // Both are configured; each light takes the one it can use,
        // rather than the whole house following whichever was set last.
        let target = build_ambient_target(
            &state(Some(40), Some(4000)),
            WHITE_ONLY,
            Some(40),
            Some(2200),
            Some([255, 128, 0]),
        )
        .expect("should publish");
        assert_eq!(target.color_temp_kelvin, Some(2200));
        assert_eq!(target.rgb, None, "it has no colour channel to send to");
    }

    #[test]
    fn a_colour_light_gets_the_colour_and_not_both() {
        let target = build_ambient_target(
            &state(Some(40), Some(4000)),
            RGB,
            Some(40),
            Some(2200),
            Some([255, 128, 0]),
        )
        .expect("should publish");
        assert_eq!(target.rgb, Some([255, 128, 0]));
        assert_eq!(
            target.color_temp_kelvin, None,
            "a light is in one mode or the other"
        );
    }

    #[test]
    fn ambient_leaves_unset_fields_alone() {
        // Brightness only: whatever colour it is showing stays.
        let target =
            build_ambient_target(&state(Some(100), Some(2700)), RGB, Some(40), None, None).unwrap();
        assert_eq!(target.brightness, Some(40));
        assert_eq!(target.color_temp_kelvin, None);
        assert_eq!(target.rgb, None);
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

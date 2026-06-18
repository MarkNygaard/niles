//! Pure-logic classifier mapping Z2M action strings (e.g.
//! `"on_press"`, `"up_hold"`) into a `SwitchEffect` enum the
//! binary can act on. No I/O, no async — the caller wires the
//! effect to whatever publish path it owns.
//!
//! Step sizes:
//! - tap (`*_press`): ±25%
//! - hold-repeat (`*_hold`, re-emitted by Z2M ~once/0.8s while held): ±12%
//!
//! `_release` variants are deliberately mapped to `None` — the
//! leading-edge events (`*_press`) and per-repeat events
//! (`*_hold`) have already driven the effect.
//!
//! Two device vocabularies are handled:
//! - **Hue dimmer (RWL, 4 buttons):** `on_press` / `off_press` /
//!   `up_*` / `down_*`.
//! - **Hue Smart Button (ROM001, 1 button):** bare `on` / `off`,
//!   which the button alternates on successive presses. Its
//!   `press` / `release` / `hold` fire on *every* press regardless
//!   of toggle direction, so they're ignored (→ `None`) to avoid
//!   double-acting with the `on` / `off` it emits alongside.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchEffect {
    /// Turn on every device in the switch's room.
    TurnOnRoom,
    /// Turn off every device in the switch's room.
    TurnOffRoom,
    /// Step the brightness of every device in the room by
    /// `delta_percent`. Positive = brighter. Caller clamps to 0..=100.
    StepBrightness { delta_percent: i16 },
}

/// Map a Z2M-format action string to a `SwitchEffect`. Returns
/// `None` for actions we deliberately ignore (`_release` variants,
/// unknown strings, empty).
pub fn classify_action(action: &str) -> Option<SwitchEffect> {
    match action {
        // Hue Smart Button (1-button) toggles with bare on/off.
        "on" => Some(SwitchEffect::TurnOnRoom),
        "off" => Some(SwitchEffect::TurnOffRoom),
        // Hue dimmer (RWL, 4-button).
        "on_press" => Some(SwitchEffect::TurnOnRoom),
        "off_press" => Some(SwitchEffect::TurnOffRoom),
        "up_press" => Some(SwitchEffect::StepBrightness { delta_percent: 25 }),
        "down_press" => Some(SwitchEffect::StepBrightness { delta_percent: -25 }),
        "up_hold" => Some(SwitchEffect::StepBrightness { delta_percent: 12 }),
        "down_hold" => Some(SwitchEffect::StepBrightness { delta_percent: -12 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_press_turns_on_room() {
        assert_eq!(classify_action("on_press"), Some(SwitchEffect::TurnOnRoom));
    }

    #[test]
    fn off_press_turns_off_room() {
        assert_eq!(
            classify_action("off_press"),
            Some(SwitchEffect::TurnOffRoom)
        );
    }

    #[test]
    fn up_press_steps_25() {
        assert_eq!(
            classify_action("up_press"),
            Some(SwitchEffect::StepBrightness { delta_percent: 25 })
        );
    }

    #[test]
    fn down_press_steps_minus_25() {
        assert_eq!(
            classify_action("down_press"),
            Some(SwitchEffect::StepBrightness { delta_percent: -25 })
        );
    }

    #[test]
    fn up_hold_steps_12() {
        assert_eq!(
            classify_action("up_hold"),
            Some(SwitchEffect::StepBrightness { delta_percent: 12 })
        );
    }

    #[test]
    fn down_hold_steps_minus_12() {
        assert_eq!(
            classify_action("down_hold"),
            Some(SwitchEffect::StepBrightness { delta_percent: -12 })
        );
    }

    #[test]
    fn press_release_variants_are_none() {
        assert!(classify_action("on_press_release").is_none());
        assert!(classify_action("off_press_release").is_none());
        assert!(classify_action("up_press_release").is_none());
        assert!(classify_action("down_press_release").is_none());
    }

    #[test]
    fn hold_release_variants_are_none() {
        assert!(classify_action("up_hold_release").is_none());
        assert!(classify_action("down_hold_release").is_none());
    }

    #[test]
    fn hue_smart_button_bare_on_off() {
        // The 1-button Hue Smart Button alternates bare on/off.
        assert_eq!(classify_action("on"), Some(SwitchEffect::TurnOnRoom));
        assert_eq!(classify_action("off"), Some(SwitchEffect::TurnOffRoom));
    }

    #[test]
    fn hue_smart_button_press_release_hold_ignored() {
        // These fire on every press regardless of toggle direction, so
        // they must not double-act with the on/off emitted alongside.
        assert!(classify_action("press").is_none());
        assert!(classify_action("release").is_none());
        assert!(classify_action("hold").is_none());
        assert!(classify_action("hold_release").is_none());
    }

    #[test]
    fn unknown_action_is_none() {
        assert!(classify_action("garbage").is_none());
    }

    #[test]
    fn empty_action_is_none() {
        assert!(classify_action("").is_none());
    }
}

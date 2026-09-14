//! Source-aware command router.
//!
//! Today every "set a light" path calls `format_set_command(z2m_prefix, …)`.
//! `CommandRouter` dispatches on `DeviceId::source()` so WLED devices get
//! WLED-formatted `/api` commands while Z2M devices continue to use the
//! existing `/set` topic.

use crate::sink::{format_set_command_fading, is_actionable};
use crate::wled::{effect_to_fx, format_wled_command_fading, format_wled_effect};
use niles_core::{DeviceId, DeviceState};
use std::collections::HashMap;
use std::time::Duration;

/// Routes set commands to the correct topic/payload format based on the
/// device source.
#[derive(Debug, Clone)]
pub struct CommandRouter {
    z2m_prefix: String,
    wled: HashMap<DeviceId, String>, // id → base topic
}

impl CommandRouter {
    pub fn new(z2m_prefix: impl Into<String>, wled: HashMap<DeviceId, String>) -> Self {
        Self {
            z2m_prefix: z2m_prefix.into(),
            wled,
        }
    }

    /// Convenience constructor for Z2M-only setups.
    pub fn z2m_only(z2m_prefix: impl Into<String>) -> Self {
        Self::new(z2m_prefix, HashMap::new())
    }

    /// Source-aware formatting. Returns `None` for a no-op target or an
    /// unknown/unsupported device.
    pub fn format(&self, id: &DeviceId, target: &DeviceState) -> Option<(String, String)> {
        self.format_fading(id, target, Duration::ZERO)
    }

    /// The same, but telling the light to take `fade` getting there.
    ///
    /// Only the two ramps ask for one. A voice command, a tap on the
    /// dashboard and the wall dimmer all go through [`format`](Self::format)
    /// and stay instant: a control that answers in its own time reads as
    /// broken, however pretty the fade.
    pub fn format_fading(
        &self,
        id: &DeviceId,
        target: &DeviceState,
        fade: Duration,
    ) -> Option<(String, String)> {
        match id.source() {
            "wled" => self
                .wled
                .get(id)
                .and_then(|base_topic| format_wled_command_fading(base_topic, target, fade)),
            "z2m" => is_actionable(target)
                .then(|| format_set_command_fading(&self.z2m_prefix, id, target, fade)),
            _ => None,
        }
    }

    /// WLED-only: map a curated effect name to its FX index and format the
    /// `/api` command. Returns `None` for a non-WLED device or unknown effect.
    pub fn format_effect(&self, id: &DeviceId, effect: &str) -> Option<(String, String)> {
        match id.source() {
            "wled" => {
                let base_topic = self.wled.get(id)?;
                let fx = effect_to_fx(effect)?;
                Some(format_wled_effect(base_topic, fx))
            }
            _ => None,
        }
    }

    pub fn z2m_prefix(&self) -> &str {
        &self.z2m_prefix
    }
}

/// What a device will never tell us it did.
///
/// WLED publishes a brightness and a colour and nothing else, so a
/// colour temperature sent to one has no echo. Nothing ever updates the
/// registry, which has two consequences and both of them look like
/// breakage: the slider in the app snaps back to its default the moment
/// you let go, and the curve finds the strip off its colour temperature
/// on every single tick — for ever, once a minute, taking it back from
/// anyone who set it by hand.
///
/// So the caller records what it sent, for the fields that have no way
/// of coming back. Only those: a Zigbee bulb reports its own state, and
/// claiming a value there would be inventing one where a real answer is
/// already on its way.
pub fn unechoed(id: &DeviceId, sent: &DeviceState) -> Option<DeviceState> {
    if id.source() != "wled" {
        return None;
    }
    sent.color_temp_kelvin.map(|kelvin| DeviceState {
        color_temp_kelvin: Some(kelvin),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{DeviceName, DeviceState, RoomName};

    fn wled_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "wled",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    fn z2m_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn a_wled_colour_temperature_has_to_be_remembered() {
        // WLED publishes a brightness and a colour and nothing else, so
        // this one never comes back. Without recording it the slider
        // snaps back the moment you let go, and the curve finds the
        // strip off-curve every tick for ever.
        let sent = DeviceState {
            brightness: Some(80),
            color_temp_kelvin: Some(2700),
            ..Default::default()
        };
        let echo = unechoed(&wled_id("living_room", "ceiling"), &sent).expect("worth recording");
        assert_eq!(echo.color_temp_kelvin, Some(2700));
        assert_eq!(
            echo.brightness, None,
            "WLED reports this one itself; claiming it would invent a              value where a real answer is already on its way"
        );
    }

    #[test]
    fn a_zigbee_light_is_left_to_report_its_own_state() {
        let sent = DeviceState {
            color_temp_kelvin: Some(2700),
            ..Default::default()
        };
        assert!(unechoed(&z2m_id("living_room", "bulb_1"), &sent).is_none());
    }

    #[test]
    fn nothing_to_remember_when_none_was_sent() {
        let sent = DeviceState {
            brightness: Some(80),
            ..Default::default()
        };
        assert!(unechoed(&wled_id("living_room", "ceiling"), &sent).is_none());
    }

    #[test]
    fn z2m_only_routes_z2m() {
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = z2m_id("kitchen", "ceiling_light");
        let target = DeviceState {
            on: Some(true),
            ..Default::default()
        };
        let (topic, payload) = router.format(&id, &target).unwrap();
        assert_eq!(topic, "zigbee2mqtt/kitchen/ceiling_light/set");
        assert!(payload.contains("\"state\":\"ON\""));
    }

    #[test]
    fn z2m_only_returns_none_for_wled() {
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = wled_id("office", "desk_strip");
        let target = DeviceState {
            on: Some(true),
            ..Default::default()
        };
        assert!(router.format(&id, &target).is_none());
    }

    #[test]
    fn populated_router_routes_wled() {
        let mut map = HashMap::new();
        map.insert(wled_id("office", "desk_strip"), "wled/office".into());
        let router = CommandRouter::new("zigbee2mqtt", map);

        let id = wled_id("office", "desk_strip");
        let target = DeviceState {
            on: Some(true),
            brightness: Some(50),
            ..Default::default()
        };
        let (topic, payload) = router.format(&id, &target).unwrap();
        assert_eq!(topic, "wled/office/api");
        assert!(payload.contains("\"on\":true"));
        assert!(payload.contains("\"bri\":128"));
    }

    #[test]
    fn unknown_wled_returns_none() {
        let router = CommandRouter::new("zigbee2mqtt", HashMap::new());
        let id = wled_id("office", "desk_strip");
        let target = DeviceState {
            on: Some(true),
            ..Default::default()
        };
        assert!(router.format(&id, &target).is_none());
    }

    #[test]
    fn no_op_target_returns_none() {
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = z2m_id("kitchen", "ceiling_light");
        assert!(router.format(&id, &DeviceState::default()).is_none());
    }

    #[test]
    fn z2m_sends_a_colour() {
        // RGB lights are the normal case for an accent strip, and this
        // used to return None — Niles simply could not set one.
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = z2m_id("kitchen", "ceiling_light");
        let target = DeviceState {
            rgb: Some([255, 128, 0]),
            ..Default::default()
        };
        let (topic, payload) = router
            .format(&id, &target)
            .expect("an RGB target is actionable");
        assert_eq!(topic, "zigbee2mqtt/kitchen/ceiling_light/set");
        assert!(
            payload.contains(r#""color":{"r":255,"g":128,"b":0}"#),
            "{payload}"
        );
    }

    #[test]
    fn z2m_colour_wins_over_colour_temperature() {
        // A light is in colour mode or white mode, never both; sending
        // each would leave the winner up to the firmware.
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = z2m_id("kitchen", "ceiling_light");
        let target = DeviceState {
            rgb: Some([255, 128, 0]),
            color_temp_kelvin: Some(2700),
            ..Default::default()
        };
        let (_, payload) = router.format(&id, &target).unwrap();
        assert!(payload.contains(r#""color""#), "{payload}");
        assert!(!payload.contains("color_temp"), "{payload}");
    }

    #[test]
    fn unsupported_source_returns_none() {
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = DeviceId::parse("matter:kitchen/ceiling_light").unwrap();
        let target = DeviceState {
            on: Some(true),
            ..Default::default()
        };
        assert!(router.format(&id, &target).is_none());
    }
    #[test]
    fn format_effect_routes_wled() {
        let mut map = HashMap::new();
        map.insert(wled_id("office", "desk_strip"), "wled/office".into());
        let router = CommandRouter::new("zigbee2mqtt", map);

        let id = wled_id("office", "desk_strip");
        let (topic, payload) = router.format_effect(&id, "fire").unwrap();
        assert_eq!(topic, "wled/office/api");
        assert_eq!(payload, r#"{"seg":[{"fx":66}]}"#);
    }

    #[test]
    fn format_effect_returns_none_for_z2m() {
        let router = CommandRouter::z2m_only("zigbee2mqtt");
        let id = z2m_id("kitchen", "ceiling_light");
        assert!(router.format_effect(&id, "fire").is_none());
    }

    #[test]
    fn format_effect_returns_none_for_unknown_effect() {
        let mut map = HashMap::new();
        map.insert(wled_id("office", "desk_strip"), "wled/office".into());
        let router = CommandRouter::new("zigbee2mqtt", map);

        let id = wled_id("office", "desk_strip");
        assert!(router.format_effect(&id, "bogus").is_none());
    }

    #[test]
    fn format_effect_returns_none_for_unmapped_wled() {
        let router = CommandRouter::new("zigbee2mqtt", HashMap::new());
        let id = wled_id("office", "desk_strip");
        assert!(router.format_effect(&id, "fire").is_none());
    }
}

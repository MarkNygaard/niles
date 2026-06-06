//! Source-aware command router.
//!
//! Today every "set a light" path calls `format_set_command(z2m_prefix, …)`.
//! `CommandRouter` dispatches on `DeviceId::source()` so WLED devices get
//! WLED-formatted `/api` commands while Z2M devices continue to use the
//! existing `/set` topic.

use crate::sink::{format_set_command, is_actionable};
use crate::wled::format_wled_command;
use niles_core::{DeviceId, DeviceState};
use std::collections::HashMap;

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
        match id.source() {
            "wled" => self
                .wled
                .get(id)
                .and_then(|base_topic| format_wled_command(base_topic, target)),
            _ => is_actionable(target).then(|| format_set_command(&self.z2m_prefix, id, target)),
        }
    }

    pub fn z2m_prefix(&self) -> &str {
        &self.z2m_prefix
    }
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
}

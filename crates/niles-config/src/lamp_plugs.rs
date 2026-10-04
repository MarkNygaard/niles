//! Which smart plugs have a lamp on them.

use crate::ambient_lights::parse_device;
use crate::error::Result;
use niles_core::DeviceId;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

/// `[lamp_plugs]` section of the config file.
///
/// A plug says it is an outlet and nothing about what is plugged into
/// it. Listed here, it is a lamp: "lights off", a room's lights, a wall
/// switch, the morning routine and scenes all include it. Not listed,
/// it is only ever switched by name, so the lights going off never
/// takes the router or the fridge with them.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LampPlugsConfig {
    #[serde(default)]
    pub devices: Vec<String>,
    /// [`Self::ids`] memoized per config snapshot, as with ambient lights.
    #[serde(skip)]
    parsed: OnceLock<HashSet<DeviceId>>,
}

impl LampPlugsConfig {
    /// The listed plugs as a set, parsed once per config snapshot. A bare
    /// `room/device` means Zigbee, as it does for ambient lights.
    pub fn ids(&self) -> &HashSet<DeviceId> {
        self.parsed.get_or_init(|| {
            self.devices
                .iter()
                .filter_map(|raw| parse_device("lamp_plugs", raw).ok())
                .collect()
        })
    }

    pub fn validate(&self) -> Result<()> {
        for raw in &self.devices {
            parse_device("lamp_plugs", raw)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_listed_is_no_lamps() {
        let cfg: LampPlugsConfig = toml::from_str("").unwrap();
        assert!(cfg.ids().is_empty());
        cfg.validate().unwrap();
    }

    #[test]
    fn a_bare_id_means_zigbee() {
        let cfg: LampPlugsConfig =
            toml::from_str(r#"devices = ["living_room/corner_lamp"]"#).unwrap();
        let id = DeviceId::parse("z2m:living_room/corner_lamp").unwrap();
        assert!(cfg.ids().contains(&id));
    }

    #[test]
    fn a_bad_id_names_the_section() {
        let cfg: LampPlugsConfig = toml::from_str(r#"devices = ["nonsense"]"#).unwrap();
        let msg = cfg.validate().unwrap_err().to_string();
        assert!(
            msg.contains("lamp_plugs") && msg.contains("nonsense"),
            "{msg}"
        );
    }
}

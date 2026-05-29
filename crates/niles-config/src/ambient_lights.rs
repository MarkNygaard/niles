//! Ambient lights opt-out configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[ambient_lights]` section of the config file.
///
/// Optional. Devices listed here are excluded from the ambient
/// lighting curve and the morning routine, while all other
/// subsystems (voice, switch, scenes, HTTP API) continue to treat
/// them as normal lights.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AmbientLightsConfig {
    #[serde(default)]
    pub devices: Vec<String>,
}

impl AmbientLightsConfig {
    pub fn validate(&self) -> Result<()> {
        for raw in &self.devices {
            if let Err(e) = niles_core::DeviceId::parse(&format!("z2m:{raw}")) {
                return Err(Error::InvalidSection {
                    section: "ambient_lights",
                    reason: format!("device {raw:?} is not a valid `<room>/<device>` id: {e}"),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_empty_parses() {
        let cfg: AmbientLightsConfig = toml::from_str("").unwrap();
        assert!(cfg.devices.is_empty());
        cfg.validate().unwrap();
    }

    #[test]
    fn valid_device_ids_parse() {
        let cfg: AmbientLightsConfig = toml::from_str(
            r#"
            devices = ["living_room/tv_lightstrip", "bedroom/led_strip"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.devices.len(), 2);
        cfg.validate().unwrap();
    }

    #[test]
    fn invalid_device_id_rejected() {
        let cfg: AmbientLightsConfig = toml::from_str(
            r#"
            devices = ["nonsense"]
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("ambient_lights"),
            "error should mention section name: {msg}"
        );
        assert!(
            msg.contains("nonsense"),
            "error should mention the offending id: {msg}"
        );
    }
}

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
    /// The configured devices, parsed.
    ///
    /// The single place an entry becomes a [`DeviceId`]: callers that
    /// re-derived it were how a WLED light ended up unable to be
    /// ambient at all.
    pub fn device_ids(&self) -> Result<Vec<niles_core::DeviceId>> {
        self.devices.iter().map(|raw| parse_device(raw)).collect()
    }

    pub fn validate(&self) -> Result<()> {
        self.device_ids().map(|_| ())
    }
}

/// Parse one entry, which may or may not name its source.
///
/// A bare `room/device` means Zigbee — every light was Zigbee when this
/// section was added, and that spelling stays valid. A device from any
/// other source names it: `wled:living_room/tv_light`.
fn parse_device(raw: &str) -> Result<niles_core::DeviceId> {
    let qualified = if raw.contains(':') {
        raw.to_string()
    } else {
        format!("z2m:{raw}")
    };
    niles_core::DeviceId::parse(&qualified).map_err(|e| Error::InvalidSection {
        section: "ambient_lights",
        reason: format!("device {raw:?} is not a valid device id: {e}"),
    })
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
    fn a_bare_id_still_means_zigbee() {
        let cfg: AmbientLightsConfig =
            toml::from_str(r#"devices = ["living_room/tv_lightstrip"]"#).unwrap();
        let ids = cfg.device_ids().unwrap();
        assert_eq!(ids[0].to_string(), "z2m:living_room/tv_lightstrip");
    }

    #[test]
    fn a_device_from_another_source_can_name_it() {
        // Without this a WLED strip could be listed here and silently
        // never be treated as ambient: the id it was compared against
        // was always a `z2m:` one.
        let cfg: AmbientLightsConfig =
            toml::from_str(r#"devices = ["wled:living_room/tv_light"]"#).unwrap();
        let ids = cfg.device_ids().unwrap();
        assert_eq!(ids[0].to_string(), "wled:living_room/tv_light");
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

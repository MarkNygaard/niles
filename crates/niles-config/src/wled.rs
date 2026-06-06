//! WLED device source configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::HashSet;

/// `[wled]` section of the config file.
///
/// Each entry declares a WLED instance that niles should treat as a
/// light. Devices are discovered from config, not from MQTT.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WledConfig {
    #[serde(default)]
    pub devices: Vec<WledDeviceConfig>,
}

/// A single WLED device declaration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WledDeviceConfig {
    /// `<room>/<device>` identifier, e.g. `"office/desk_strip"`.
    pub name: String,
    /// Base MQTT topic for this WLED instance, e.g. `"wled/office"`.
    pub topic: String,
}

impl WledConfig {
    /// Helper to build an `InvalidSection` error for the `wled` section.
    fn invalid(reason: impl Into<String>) -> Error {
        Error::InvalidSection {
            section: "wled",
            reason: reason.into(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen_names = HashSet::new();
        let mut seen_topics = HashSet::new();

        for dev in &self.devices {
            if let Err(e) = niles_core::DeviceId::parse(&format!("wled:{}", dev.name)) {
                return Err(Self::invalid(format!(
                    "device name {:?} is not a valid `<room>/<device>` id: {e}",
                    dev.name
                )));
            }
            if dev.topic.trim().is_empty() {
                return Err(Self::invalid(format!(
                    "topic for {:?} must not be empty",
                    dev.name
                )));
            }
            if dev.topic.contains('#') || dev.topic.contains('+') {
                return Err(Self::invalid(format!(
                    "topic {:?} must not contain '#' or '+'",
                    dev.topic
                )));
            }
            if !seen_names.insert(dev.name.clone()) {
                return Err(Self::invalid(format!(
                    "duplicate device name {:?}",
                    dev.name
                )));
            }
            if !seen_topics.insert(dev.topic.clone()) {
                return Err(Self::invalid(format!("duplicate topic {:?}", dev.topic)));
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
        let cfg: WledConfig = toml::from_str("").unwrap();
        assert!(cfg.devices.is_empty());
        cfg.validate().unwrap();
    }

    #[test]
    fn valid_entry_validates() {
        let cfg: WledConfig = toml::from_str(
            r#"
            [[devices]]
            name = "office/desk_strip"
            topic = "wled/office"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.devices.len(), 1);
        cfg.validate().unwrap();
    }

    #[test]
    fn invalid_name_rejected() {
        let cfg: WledConfig = toml::from_str(
            r#"
            [[devices]]
            name = "Office/Strip"
            topic = "wled/office"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("wled"), "error should mention section: {msg}");
        assert!(
            msg.contains("Office/Strip"),
            "error should mention bad name: {msg}"
        );
    }

    #[test]
    fn topic_with_wildcard_rejected() {
        let cfg: WledConfig = toml::from_str(
            r#"
            [[devices]]
            name = "office/desk_strip"
            topic = "wled/+/office"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains('#') || msg.contains('+'),
            "error should mention wildcard: {msg}"
        );
    }

    #[test]
    fn duplicate_name_rejected() {
        let cfg: WledConfig = toml::from_str(
            r#"
            [[devices]]
            name = "office/desk_strip"
            topic = "wled/office1"
            [[devices]]
            name = "office/desk_strip"
            topic = "wled/office2"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("duplicate"),
            "error should mention duplicate: {msg}"
        );
    }

    #[test]
    fn duplicate_topic_rejected() {
        let cfg: WledConfig = toml::from_str(
            r#"
            [[devices]]
            name = "office/desk_strip"
            topic = "wled/office"
            [[devices]]
            name = "bedroom/strip"
            topic = "wled/office"
            "#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("duplicate"),
            "error should mention duplicate: {msg}"
        );
    }
}

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

    /// True for a strip with colour LEDs.
    ///
    /// Nearly all of them have, which is why it defaults to true — but
    /// not the analog ones, and offering a colour wheel for a strip
    /// that only does warm-to-cold white is how a control ends up
    /// doing nothing.
    #[serde(default = "default_rgb")]
    pub rgb: bool,

    /// True for a strip with warm and cold white channels — the
    /// "white balance" slider in WLED's own interface, `cct` in its
    /// API.
    ///
    /// Declared rather than detected: WLED has no `bridge/devices` to
    /// ask, and guessing from what a strip has happened to report is
    /// the mistake #167 fixed for Zigbee. Off by default, because a
    /// plain RGB strip told to warm up does nothing and looks broken
    /// rather than unsupported.
    #[serde(default)]
    pub white_balance: bool,
}

/// Nearly every WLED strip has colour LEDs; the analog ones are the
/// exception.
fn default_rgb() -> bool {
    true
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
            // A strip with neither is a light Niles can switch on and
            // dim and nothing else. That is a real thing to own, but it
            // is almost always a mistake in the entry, and saying so
            // costs less than an evening wondering why a slider does
            // nothing.
            if !dev.rgb && !dev.white_balance {
                return Err(Self::invalid(format!(
                    "{:?} has neither colour nor white balance, so nothing about                      its light can be set",
                    dev.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_is_a_colour_strip_unless_told_otherwise() {
        let cfg: WledConfig = toml::from_str(
            "[[devices]]
name = \"office/desk\"
topic = \"wled/office\"
",
        )
        .expect("parses");
        assert!(cfg.devices[0].rgb);
        assert!(!cfg.devices[0].white_balance);
        cfg.validate().expect("valid");
    }

    #[test]
    fn an_analog_strip_can_say_it_has_no_colour() {
        let cfg: WledConfig = toml::from_str(
            "[[devices]]
name = \"living_room/ceiling\"
topic = \"wled/living_room\"
             rgb = false
white_balance = true
",
        )
        .expect("parses");
        assert!(!cfg.devices[0].rgb);
        assert!(cfg.devices[0].white_balance);
        cfg.validate().expect("valid");
    }

    #[test]
    fn a_strip_with_no_channel_at_all_is_refused() {
        let cfg: WledConfig = toml::from_str(
            "[[devices]]
name = \"office/desk\"
topic = \"wled/office\"
rgb = false
",
        )
        .expect("it parses");
        let err = cfg.validate().expect_err("but does not validate");
        assert!(
            err.to_string().contains("neither colour nor white"),
            "{err}"
        );
    }

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

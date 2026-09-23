//! `[capture]` — keeping the audio behind a wake.

use crate::error::{Error, Result};
use serde::Deserialize;

/// Whether to keep what the satellite heard.
///
/// A satellite woke fifty-three times in one day and meant it twice.
/// Every one of those was a recording of the room — the television, a
/// word that sounded enough like the wake word — transcribed and
/// thrown away. They are the exact distribution the wake word is
/// failing on, which makes them worth more for training than any
/// number of synthesised negatives.
///
/// **Off by default, and it stays off until somebody switches it on.**
/// This is a microphone writing down a living room, and no amount of
/// usefulness makes that a reasonable default.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfig {
    #[serde(default)]
    pub enabled: bool,
    /// How many recordings to keep. The oldest go first.
    ///
    /// A cap rather than a duration: a house that wakes twice a day
    /// and one that wakes fifty times should both end up with enough
    /// to train on and neither should fill a database.
    #[serde(default = "default_keep")]
    pub keep: u32,
}

fn default_keep() -> u32 {
    500
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keep: default_keep(),
        }
    }
}

impl CaptureConfig {
    pub fn validate(&self) -> Result<()> {
        // A cap of zero would keep nothing while claiming to be on,
        // which reads as a broken feature rather than a disabled one.
        if self.enabled && self.keep == 0 {
            return Err(Error::InvalidSection {
                section: "capture",
                reason: "keep must be at least 1 when capture is enabled".into(),
            });
        }
        if self.keep > 10_000 {
            return Err(Error::InvalidSection {
                section: "capture",
                reason: "keep above 10000 is more audio than this is for".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_microphone_is_off_until_somebody_says_otherwise() {
        assert!(!CaptureConfig::default().enabled);
        assert!(CaptureConfig::default().validate().is_ok());
    }

    #[test]
    fn keeping_nothing_while_switched_on_is_refused() {
        let cfg = CaptureConfig {
            enabled: true,
            keep: 0,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn a_cap_nobody_could_want_is_refused() {
        let cfg = CaptureConfig {
            enabled: true,
            keep: 50_000,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn it_is_read_off_the_config() {
        let cfg: CaptureConfig = toml::from_str("enabled = true\nkeep = 200").expect("valid");
        assert!(cfg.enabled);
        assert_eq!(cfg.keep, 200);
    }
}

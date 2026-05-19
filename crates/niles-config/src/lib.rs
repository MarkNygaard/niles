//! niles-config — TOML loading and validation for the Niles service.
//!
//! Per-subsystem configs (`HomeConfig`, `LightingConfig`, etc.) each
//! own their own schema and validation. The top-level `Config` is a
//! container that delegates `validate()` to each subsystem.
//!
//! Currently covered sections: `[home]`, `[lighting]`. New sections
//! land alongside the crates that consume them.

pub mod error;
pub mod home;
pub mod lighting;

use serde::Deserialize;
use std::path::Path;

pub use error::{Error, Result};
pub use home::HomeConfig;
pub use lighting::{ColorTempAnchor, LightingConfig};

/// Top-level Niles configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub home: HomeConfig,
    pub lighting: LightingConfig,
}

impl Config {
    /// Parse from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(s)?;
        Ok(cfg)
    }

    /// Read and parse a TOML file.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    /// Validate every subsection. Returns the first error encountered.
    pub fn validate(&self) -> Result<()> {
        self.home.validate()?;
        let _ = self.lighting.to_curve_config()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> &'static str {
        r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[lighting]
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "05:45"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "06:30"
kelvin = 2700

[[lighting.color_temp_anchors]]
time = "12:00"
kelvin = 4500

[[lighting.color_temp_anchors]]
time = "21:30"
kelvin = 2700

[[lighting.color_temp_anchors]]
time = "23:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#
    }

    #[test]
    fn loads_and_validates_a_full_config() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.home.name, "test home");
        assert_eq!(cfg.home.latitude, 56.1572);
        assert_eq!(cfg.lighting.color_temp_anchors.len(), 7);
        let curve = cfg.lighting.to_curve_config().unwrap();
        assert_eq!(curve.night_floor_brightness, 15);
        assert_eq!(curve.color_temp_anchors.len(), 7);
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let bad = format!(
            "{}\n[unknown]\nfoo = 1\n",
            valid_toml().trim_end_matches('\n')
        );
        assert!(Config::load_from_str(&bad).is_err());
    }

    #[test]
    fn rejects_unknown_home_field() {
        let bad = valid_toml().replace(
            "[home]\nname = \"test home\"",
            "[home]\nname = \"test home\"\nunexpected = 42",
        );
        assert!(Config::load_from_str(&bad).is_err());
    }

    #[test]
    fn rejects_invalid_latitude() {
        let bad = valid_toml().replace("latitude = 56.1572", "latitude = 100.0");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_invalid_longitude() {
        let bad = valid_toml().replace("longitude = 10.2107", "longitude = -200.0");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_home_name() {
        let bad = valid_toml().replace("name = \"test home\"", "name = \"\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_malformed_time() {
        let bad = valid_toml().replace("morning_start = \"05:45\"", "morning_start = \"25:99\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        // TOML parses fine — validation fails.
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_curve_validation_failure() {
        // Inverted morning ramp ordering.
        let bad = valid_toml().replace("morning_start = \"05:45\"", "morning_start = \"07:00\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_color_temp_out_of_range() {
        let bad = valid_toml().replace("kelvin = 4500", "kelvin = 15000");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_invalid_toml_syntax() {
        let bad = "not = valid = toml";
        assert!(Config::load_from_str(bad).is_err());
    }

    #[test]
    fn parse_error_is_distinct_from_validation_error() {
        // Distinguishing them helps callers report sensibly.
        let parse_err = Config::load_from_str("not = valid = toml").unwrap_err();
        assert!(matches!(parse_err, Error::Parse(_)));

        let bad = valid_toml().replace("latitude = 56.1572", "latitude = 999.0");
        let cfg = Config::load_from_str(&bad).unwrap();
        let validate_err = cfg.validate().unwrap_err();
        assert!(matches!(
            validate_err,
            Error::InvalidSection {
                section: "home",
                ..
            }
        ));
    }
}

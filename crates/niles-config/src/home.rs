//! Home location and identity configuration.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[home]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeConfig {
    /// Display name of the home (e.g. `"Mark's apartment"`).
    pub name: String,
    /// Latitude in decimal degrees, `-90..=90`.
    pub latitude: f64,
    /// Longitude in decimal degrees, `-180..=180`.
    pub longitude: f64,
    /// IANA timezone identifier (e.g. `"Europe/Copenhagen"`).
    pub timezone: String,
}

impl HomeConfig {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "home",
                reason: "name must not be empty".into(),
            });
        }
        if !(-90.0..=90.0).contains(&self.latitude) {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!("latitude {} outside -90..=90", self.latitude),
            });
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!("longitude {} outside -180..=180", self.longitude),
            });
        }
        if self.timezone.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "home",
                reason: "timezone must not be empty".into(),
            });
        }
        Ok(())
    }
}

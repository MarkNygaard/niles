//! Wyoming voice-protocol server configuration.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::net::SocketAddr;
use std::str::FromStr;

/// `[wyoming]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WyomingConfig {
    /// Address the Wyoming server binds to (e.g. `"0.0.0.0:10300"`).
    /// 10300 is the conventional Wyoming port used by Home Assistant
    /// satellites and ESPHome's voice_assistant component.
    pub bind_address: String,
}

impl WyomingConfig {
    pub fn validate(&self) -> Result<()> {
        let _ = self.socket_addr()?;
        Ok(())
    }

    /// Parse `bind_address` into a `SocketAddr`.
    pub fn socket_addr(&self) -> Result<SocketAddr> {
        SocketAddr::from_str(&self.bind_address).map_err(|e| Error::InvalidSection {
            section: "wyoming",
            reason: format!("bind_address {:?}: {e}", self.bind_address),
        })
    }
}

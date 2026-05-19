//! HTTP API server configuration.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::net::SocketAddr;
use std::str::FromStr;

/// `[api]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// Address to bind the HTTP server to (e.g. `"0.0.0.0:8080"`).
    pub bind_address: String,
}

impl ApiConfig {
    pub fn validate(&self) -> Result<()> {
        let _ = self.socket_addr()?;
        Ok(())
    }

    /// Parse `bind_address` into a `SocketAddr`. Use this when
    /// handing the config to `niles_api::serve`.
    pub fn socket_addr(&self) -> Result<SocketAddr> {
        SocketAddr::from_str(&self.bind_address).map_err(|e| Error::InvalidSection {
            section: "api",
            reason: format!("bind_address {:?}: {e}", self.bind_address),
        })
    }
}

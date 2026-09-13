//! MQTT broker configuration.
//!
//! The TOML carries the broker address plus *names* of env vars that
//! hold the credentials — never the credentials themselves. This is
//! the architecture-wide secrets pattern (config in TOML, secrets in
//! env / `.env` for local dev, k8s `Secret` in cluster deploy).

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[mqtt]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttConfig {
    /// Broker host (IP or DNS name).
    #[serde(default)]
    pub host: String,
    /// Broker TCP port (typically 1883 unplain, 8883 TLS).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Name of the env var holding the broker username.
    #[serde(default)]
    pub username_env: String,
    /// Name of the env var holding the broker password.
    #[serde(default)]
    pub password_env: String,
    /// MQTT client identifier. Defaults to `"niles"`.
    #[serde(default = "default_client_id")]
    pub client_id: String,
    /// Zigbee2MQTT topic prefix. Defaults to `"zigbee2mqtt"`.
    #[serde(default = "default_z2m_prefix")]
    pub z2m_prefix: String,
}

fn default_client_id() -> String {
    "niles".into()
}

fn default_z2m_prefix() -> String {
    "zigbee2mqtt".into()
}

fn default_port() -> u16 {
    1883
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: default_port(),
            username_env: String::new(),
            password_env: String::new(),
            client_id: default_client_id(),
            z2m_prefix: default_z2m_prefix(),
        }
    }
}

impl MqttConfig {
    pub fn validate(&self) -> Result<()> {
        // An empty host is *unset*, not wrong, and is reported by
        // `Config::setup_gaps` instead. Failing here would mean a fresh
        // Niles could not start far enough to be told the answer.
        if self.port == 0 {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: "port must not be 0".into(),
            });
        }
        if self.z2m_prefix.contains('/')
            || self.z2m_prefix.contains('#')
            || self.z2m_prefix.contains('+')
        {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: format!(
                    "z2m_prefix '{}' must not contain '/', '#', or '+' \
                     (it's a topic root, not a pattern)",
                    self.z2m_prefix
                ),
            });
        }
        Ok(())
    }

    /// Read the username + password from the environment variables
    /// named by `username_env` / `password_env`. Returns an
    /// `InvalidSection` error if either is unset.
    pub fn resolve_credentials(&self) -> Result<(String, String)> {
        let username = crate::env::require_env("mqtt", &self.username_env)?;
        let password = crate::env::require_env("mqtt", &self.password_env)?;
        Ok((username, password))
    }
}

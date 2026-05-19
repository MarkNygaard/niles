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
    pub host: String,
    /// Broker TCP port (typically 1883 unplain, 8883 TLS).
    pub port: u16,
    /// Name of the env var holding the broker username.
    pub username_env: String,
    /// Name of the env var holding the broker password.
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

impl MqttConfig {
    pub fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: "host must not be empty".into(),
            });
        }
        if self.port == 0 {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: "port must not be 0".into(),
            });
        }
        if self.username_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: "username_env must not be empty".into(),
            });
        }
        if self.password_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "mqtt",
                reason: "password_env must not be empty".into(),
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
        let username = std::env::var(&self.username_env).map_err(|_| Error::InvalidSection {
            section: "mqtt",
            reason: format!("env var {} is not set", self.username_env),
        })?;
        let password = std::env::var(&self.password_env).map_err(|_| Error::InvalidSection {
            section: "mqtt",
            reason: format!("env var {} is not set", self.password_env),
        })?;
        Ok((username, password))
    }
}

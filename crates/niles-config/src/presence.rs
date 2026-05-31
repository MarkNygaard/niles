//! Presence configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_poll_seconds() -> u64 {
    60
}

fn default_away_debounce_minutes() -> u64 {
    5
}

fn default_tado_base_url() -> String {
    "https://my.tado.com".into()
}

/// `[presence.tado]` subsection.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TadoConfigDto {
    pub username_env: String,
    pub password_env: String,
    pub home_id: u64,
    #[serde(default = "default_tado_base_url")]
    pub base_url: String,
}

impl Default for TadoConfigDto {
    fn default() -> Self {
        Self {
            username_env: String::new(),
            password_env: String::new(),
            home_id: 0,
            base_url: default_tado_base_url(),
        }
    }
}

impl TadoConfigDto {
    pub fn resolve_env(&self) -> Result<(String, String)> {
        let username = std::env::var(&self.username_env).map_err(|_| Error::InvalidSection {
            section: "presence.tado",
            reason: format!("env var {} not set", self.username_env),
        })?;
        let password = std::env::var(&self.password_env).map_err(|_| Error::InvalidSection {
            section: "presence.tado",
            reason: format!("env var {} not set", self.password_env),
        })?;
        Ok((username, password))
    }
}

/// `[presence]` section of the config file.
///
/// Optional. If absent, presence features are disabled.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(default = "default_away_debounce_minutes")]
    pub away_debounce_minutes: u64,
    #[serde(default)]
    pub tado: Option<TadoConfigDto>,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_seconds: default_poll_seconds(),
            away_debounce_minutes: default_away_debounce_minutes(),
            tado: None,
        }
    }
}

impl PresenceConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.tado.is_none() {
            return Err(Error::InvalidSection {
                section: "presence",
                reason: "no adapter configured".into(),
            });
        }
        if !(10..=3600).contains(&self.poll_seconds) {
            return Err(Error::InvalidSection {
                section: "presence",
                reason: "poll_seconds must be in 10..=3600".into(),
            });
        }
        if self.away_debounce_minutes > 120 {
            return Err(Error::InvalidSection {
                section: "presence",
                reason: "away_debounce_minutes must be <= 120".into(),
            });
        }
        if let Some(tado) = &self.tado {
            if tado.username_env.is_empty() {
                return Err(Error::InvalidSection {
                    section: "presence.tado",
                    reason: "username_env must not be empty".into(),
                });
            }
            if tado.password_env.is_empty() {
                return Err(Error::InvalidSection {
                    section: "presence.tado",
                    reason: "password_env must not be empty".into(),
                });
            }
            if tado.home_id == 0 {
                return Err(Error::InvalidSection {
                    section: "presence.tado",
                    reason: "home_id must be > 0".into(),
                });
            }
            if !tado.base_url.starts_with("http://") && !tado.base_url.starts_with("https://") {
                return Err(Error::InvalidSection {
                    section: "presence.tado",
                    reason: "base_url must start with http:// or https://".into(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_disabled() {
        let cfg = PresenceConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.poll_seconds, 60);
        assert_eq!(cfg.away_debounce_minutes, 5);
        assert!(cfg.tado.is_none());
    }

    #[test]
    fn enabled_without_source_fails_validation() {
        let cfg = PresenceConfig {
            enabled: true,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence",
                reason,
            } if reason.contains("no adapter")
        ));
    }

    #[test]
    fn tado_parses_with_defaults() {
        let toml = r#"
enabled = true
[tado]
username_env = "TADO_USER"
password_env = "TADO_PASS"
home_id = 123
"#;
        let cfg: PresenceConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        let tado = cfg.tado.as_ref().unwrap();
        assert_eq!(tado.username_env, "TADO_USER");
        assert_eq!(tado.base_url, "https://my.tado.com");
    }

    #[test]
    fn validate_accepts_good_config() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 60,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                username_env: "U".into(),
                password_env: "P".into(),
                home_id: 1,
                base_url: "https://my.tado.com".into(),
            }),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_poll_seconds_too_small() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 5,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto::default()),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_poll_seconds_too_large() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 4000,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto::default()),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_away_debounce_too_large() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 60,
            away_debounce_minutes: 121,
            tado: Some(TadoConfigDto::default()),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_empty_tado_username_env() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 60,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                username_env: "".into(),
                password_env: "P".into(),
                home_id: 1,
                base_url: "https://my.tado.com".into(),
            }),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence.tado",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_zero_tado_home_id() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 60,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                username_env: "U".into(),
                password_env: "P".into(),
                home_id: 0,
                base_url: "https://my.tado.com".into(),
            }),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence.tado",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_tado_base_url_without_http_scheme() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 60,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                username_env: "U".into(),
                password_env: "P".into(),
                home_id: 1,
                base_url: "my.tado.com".into(),
            }),
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence.tado",
                ..
            }
        ));
    }

    #[test]
    fn resolve_env_reads_vars() {
        unsafe {
            std::env::set_var("NILES_TEST_TADO_USER", "u");
            std::env::set_var("NILES_TEST_TADO_PASS", "p");
        }
        let dto = TadoConfigDto {
            username_env: "NILES_TEST_TADO_USER".into(),
            password_env: "NILES_TEST_TADO_PASS".into(),
            home_id: 1,
            base_url: "https://my.tado.com".into(),
        };
        let (u, p) = dto.resolve_env().unwrap();
        assert_eq!(u, "u");
        assert_eq!(p, "p");
    }

    #[test]
    fn resolve_env_errors_when_missing() {
        let dto = TadoConfigDto {
            username_env: "NILES_TEST_DEFINITELY_NOT_SET_TADO_XYZ".into(),
            password_env: "NILES_TEST_DEFINITELY_NOT_SET_TADO_XYZ".into(),
            home_id: 1,
            base_url: "https://my.tado.com".into(),
        };
        let err = dto.resolve_env().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "presence.tado",
                ..
            }
        ));
    }
}

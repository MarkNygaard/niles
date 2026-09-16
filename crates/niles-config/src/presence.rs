//! Presence configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

/// Five minutes, which is what Home Assistant's tado integration
/// settles on and roughly what tado's quota tolerates: it rate-limits
/// per day, resetting around midday Berlin time, and a minute's polling
/// spends the budget long before the day is out. Presence changes when
/// somebody walks out of a geofence, so minutes is the right grain
/// anyway.
fn default_poll_seconds() -> u64 {
    300
}

fn default_away_debounce_minutes() -> u64 {
    5
}

fn default_tado_base_url() -> String {
    "https://my.tado.com".into()
}

/// `[presence.tado]` subsection.
///
/// No credentials here on purpose. tado removed the password grant in
/// March 2025, and what replaced it is a browser approval that yields a
/// refresh token — which is state, not configuration, and lives in
/// Postgres beside the config overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TadoConfigDto {
    /// Usually left out: discovered from `/api/v2/me` on first use.
    /// Worth setting only for an account with more than one home.
    #[serde(default)]
    pub home_id: Option<u64>,
    #[serde(default = "default_tado_base_url")]
    pub base_url: String,

    /// Which Niles room each tado zone is, keyed by zone id.
    ///
    /// Set from Settings, and only where it is needed. Zone names are
    /// whatever somebody typed into the tado app years ago — "Stue",
    /// "Kids room", "Radiator hall" — and Niles room names are
    /// canonical. Where the two happen to agree, nothing needs to be
    /// written here; where they do not, guessing would attach a
    /// radiator to the wrong room and nothing would ever say so.
    ///
    /// Keyed by id rather than name so renaming a zone in the tado app
    /// does not silently unpair it.
    #[serde(default)]
    pub rooms: std::collections::HashMap<String, String>,
}

impl Default for TadoConfigDto {
    fn default() -> Self {
        Self {
            home_id: None,
            base_url: default_tado_base_url(),
            rooms: std::collections::HashMap::new(),
        }
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
    /// Turn the lights off when the last person leaves.
    ///
    /// Off by default, and deliberately: presence can be switched on
    /// to answer "is anybody home" without also handing it the light
    /// switches. Somebody has to say that this is wanted.
    #[serde(default)]
    pub lights_off_when_away: bool,
    /// The lights to turn on when somebody comes back.
    ///
    /// Named rather than "all of them", because arriving is not the
    /// same shape of event as leaving: turning everything off on the
    /// way out is what a person would do anyway, while turning
    /// everything on as they come through the door is nobody's idea of
    /// coming home. The hall and the kitchen, not the whole house.
    #[serde(default)]
    pub lights_on_when_home: Vec<String>,
    #[serde(default)]
    pub tado: Option<TadoConfigDto>,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_seconds: default_poll_seconds(),
            away_debounce_minutes: default_away_debounce_minutes(),
            lights_off_when_away: false,
            lights_on_when_home: Vec::new(),
            tado: None,
        }
    }
}

impl PresenceConfig {
    /// The lights to turn on when somebody arrives, parsed.
    ///
    /// The single place an entry becomes a `DeviceId`, so a caller
    /// cannot re-derive it differently — the bug `ambient_lights`
    /// documents from the other end.
    pub fn arrival_lights(&self) -> Result<Vec<niles_core::DeviceId>> {
        self.lights_on_when_home
            .iter()
            .map(|raw| self.light_id(raw))
            .collect()
    }

    /// An unqualified name is Zigbee, as everywhere else in the config.
    fn light_id(&self, raw: &str) -> Result<niles_core::DeviceId> {
        let qualified = if raw.contains(':') {
            raw.to_string()
        } else {
            format!("z2m:{raw}")
        };
        niles_core::DeviceId::parse(&qualified).map_err(|e| Error::InvalidSection {
            section: "presence",
            reason: format!("lights_on_when_home: {raw:?} is not a device id: {e}"),
        })
    }

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
        // Checked here rather than where they are used: a light named
        // wrongly in this list would otherwise be a thing that quietly
        // does not happen at the door, hours later, with nothing to
        // read about it.
        for light in &self.lights_on_when_home {
            self.light_id(light)?;
        }
        if let Some(tado) = &self.tado {
            // No credential checks: there are no credentials. An
            // explicit home_id of 0 is still a mistake, but leaving it
            // out is the normal case now that it is discovered.
            if tado.home_id == Some(0) {
                return Err(Error::InvalidSection {
                    section: "presence.tado",
                    reason: "home_id must be > 0, or left out to be discovered".into(),
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
        assert_eq!(cfg.poll_seconds, 300);
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
home_id = 123
"#;
        let cfg: PresenceConfig = toml::from_str(toml).unwrap();
        assert!(cfg.enabled);
        let tado = cfg.tado.as_ref().unwrap();
        assert_eq!(tado.home_id, Some(123));
        assert_eq!(tado.base_url, "https://my.tado.com");
    }

    #[test]
    fn validate_accepts_good_config() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 300,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                home_id: Some(1),
                base_url: "https://my.tado.com".into(),
                ..Default::default()
            }),
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            poll_seconds: 300,
            away_debounce_minutes: 121,
            tado: Some(TadoConfigDto::default()),
            ..Default::default()
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
    fn the_lights_to_turn_on_have_to_be_lights_that_exist() {
        // Caught here, or it is a thing that quietly does not happen at
        // the door hours later with nothing to read about it.
        let cfg = PresenceConfig {
            enabled: true,
            lights_on_when_home: vec!["not a device".into()],
            tado: Some(TadoConfigDto::default()),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("lights_on_when_home"), "{err}");
    }

    #[test]
    fn an_unqualified_light_is_zigbee_like_everywhere_else() {
        let cfg = PresenceConfig {
            lights_on_when_home: vec!["hall/lamp".into(), "wled:office/strip".into()],
            ..Default::default()
        };
        let ids = cfg.arrival_lights().expect("both parse");
        assert_eq!(ids[0].to_string(), "z2m:hall/lamp");
        assert_eq!(ids[1].to_string(), "wled:office/strip");
    }

    #[test]
    fn validate_rejects_zero_tado_home_id() {
        let cfg = PresenceConfig {
            enabled: true,
            poll_seconds: 300,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                home_id: Some(0),
                base_url: "https://my.tado.com".into(),
                ..Default::default()
            }),
            ..Default::default()
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
            poll_seconds: 300,
            away_debounce_minutes: 5,
            tado: Some(TadoConfigDto {
                home_id: Some(1),
                base_url: "my.tado.com".into(),
                ..Default::default()
            }),
            ..Default::default()
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
}

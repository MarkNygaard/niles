//! Notification subsystem configuration.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[notifications]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Maximum number of notifications to keep in the in-memory ring buffer.
    #[serde(default = "default_capacity")]
    pub capacity: usize,
    /// Optional quiet-hours configuration.
    #[serde(default)]
    pub quiet_hours: Option<QuietHoursDto>,
}

fn default_capacity() -> usize {
    50
}

/// Quiet-hours DTO deserialized from TOML.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuietHoursDto {
    /// Whether quiet hours are enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Start time in `HH:MM` format (24-hour).
    pub start: String,
    /// End time in `HH:MM` format (24-hour).
    pub end: String,
}

fn default_enabled() -> bool {
    true
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            capacity: default_capacity(),
            quiet_hours: None,
        }
    }
}

impl NotificationsConfig {
    pub fn validate(&self) -> Result<()> {
        if self.capacity == 0 {
            return Err(Error::InvalidSection {
                section: "notifications",
                reason: "capacity must be > 0".into(),
            });
        }
        if let Some(qh) = &self.quiet_hours {
            if qh.start.trim().is_empty() {
                return Err(Error::InvalidSection {
                    section: "notifications.quiet_hours",
                    reason: "start must not be empty".into(),
                });
            }
            if qh.end.trim().is_empty() {
                return Err(Error::InvalidSection {
                    section: "notifications.quiet_hours",
                    reason: "end must not be empty".into(),
                });
            }
            // Validate HH:MM parseability.
            if parse_time(&qh.start).is_none() {
                return Err(Error::InvalidSection {
                    section: "notifications.quiet_hours",
                    reason: format!("start '{}' is not a valid HH:MM time", qh.start),
                });
            }
            if parse_time(&qh.end).is_none() {
                return Err(Error::InvalidSection {
                    section: "notifications.quiet_hours",
                    reason: format!("end '{}' is not a valid HH:MM time", qh.end),
                });
            }
        }
        Ok(())
    }

    /// Convert the config DTO into a runtime `QuietHoursConfig`.
    pub fn to_quiet_hours_config(
        &self,
        timezone_str: &str,
    ) -> Option<niles_notifications::QuietHoursConfig> {
        let qh = self.quiet_hours.as_ref()?;
        let tz = timezone_str.parse::<chrono_tz::Tz>().ok()?;
        let start = parse_time(&qh.start)?;
        let end = parse_time(&qh.end)?;
        Some(niles_notifications::QuietHoursConfig {
            enabled: qh.enabled,
            window: Some(niles_notifications::QuietWindow::new(start, end)),
            timezone: Some(tz),
        })
    }
}

/// Parse `HH:MM` into a `NaiveTime`.
fn parse_time(s: &str) -> Option<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_absent() {
        let cfg = NotificationsConfig::default();
        assert_eq!(cfg.capacity, 50);
        assert!(cfg.quiet_hours.is_none());
    }

    #[test]
    fn validates_zero_capacity() {
        let cfg = NotificationsConfig {
            capacity: 0,
            quiet_hours: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validates_empty_quiet_start() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: Some(QuietHoursDto {
                enabled: true,
                start: "".into(),
                end: "07:00".into(),
            }),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validates_malformed_quiet_time() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: Some(QuietHoursDto {
                enabled: true,
                start: "not-a-time".into(),
                end: "07:00".into(),
            }),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn valid_quiet_hours_parses() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: Some(QuietHoursDto {
                enabled: true,
                start: "22:00".into(),
                end: "07:00".into(),
            }),
        };
        cfg.validate().unwrap();
        let quiet = cfg.to_quiet_hours_config("Europe/Copenhagen").unwrap();
        assert!(quiet.enabled);
        assert!(quiet.timezone.is_some());
    }

    #[test]
    fn no_quiet_hours_returns_none() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: None,
        };
        assert!(cfg.to_quiet_hours_config("Europe/Copenhagen").is_none());
    }

    #[test]
    fn invalid_timezone_returns_none() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: Some(QuietHoursDto {
                enabled: true,
                start: "22:00".into(),
                end: "07:00".into(),
            }),
        };
        assert!(cfg.to_quiet_hours_config("Not/A/Tz").is_none());
    }

    #[test]
    fn disabled_quiet_hours_returns_config_with_enabled_false() {
        let cfg = NotificationsConfig {
            capacity: 50,
            quiet_hours: Some(QuietHoursDto {
                enabled: false,
                start: "22:00".into(),
                end: "07:00".into(),
            }),
        };
        let quiet = cfg.to_quiet_hours_config("Europe/Copenhagen").unwrap();
        assert!(!quiet.enabled);
    }
}

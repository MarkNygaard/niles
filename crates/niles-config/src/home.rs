//! Home location and identity configuration.

use crate::error::{Error, Result};
use serde::Deserialize;

/// Measurement system preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// Metric / SI units (°C, km, metres, etc.).
    #[default]
    Metric,
    /// Imperial / US customary units (°F, miles, feet, etc.).
    Imperial,
}

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
    /// Locale tag — POSIX (`en_US`) or IETF BCP-47 (`en-US`).
    #[serde(default = "default_locale")]
    pub locale: String,
    /// Explicit measurement-system override.
    #[serde(default)]
    pub units: Option<Units>,
    /// ISO-3166-1 alpha-2 country override (e.g. `"US"`, `"DK"`).
    #[serde(default)]
    pub country: Option<String>,
    /// Primary language override — two-letter ISO-639-1 code (e.g. `"en"`, `"da"`).
    #[serde(default)]
    pub default_language: Option<String>,
}

fn default_locale() -> String {
    "en_US".into()
}

impl HomeConfig {
    /// Return the effective measurement system.
    ///
    /// Explicit `units` wins; otherwise US country (explicit or
    /// locale-derived) → Imperial, all other locales → Metric.
    pub fn resolved_units(&self) -> Units {
        self.units.unwrap_or_else(|| {
            if self
                .resolved_country()
                .is_some_and(|country| country.eq_ignore_ascii_case("US"))
            {
                return Units::Imperial;
            }
            Units::Metric
        })
    }

    /// Return the effective country code, upper-cased.
    ///
    /// Explicit `country` wins; otherwise derived from the second
    /// segment of `locale`.  For bare language tags (no separator)
    /// returns `None`.
    pub fn resolved_country(&self) -> Option<String> {
        self.country.clone().map(|c| c.to_uppercase()).or_else(|| {
            let tail = locale_segment(&self.locale, 1)?;
            (tail.len() == 2 && tail.chars().all(|ch| ch.is_ascii_alphabetic()))
                .then(|| tail.to_uppercase())
        })
    }

    /// Return the effective language code, lower-cased.
    ///
    /// Explicit `default_language` wins; otherwise derived from the
    /// first segment of `locale`.
    pub fn resolved_language(&self) -> String {
        self.default_language
            .clone()
            .map(|l| l.to_lowercase())
            .unwrap_or_else(|| {
                split_locale(&self.locale)
                    .map(|(head, _)| head.to_lowercase())
                    .unwrap_or_else(|| self.locale.to_lowercase())
            })
    }

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
        if let Err(e) = self.timezone.parse::<chrono_tz::Tz>() {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!("timezone '{}' is not a valid IANA zone: {e}", self.timezone),
            });
        }
        if self.locale.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "home",
                reason: "locale must not be empty".into(),
            });
        }
        if self.locale.len() > 16 {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!("locale '{}' exceeds 16 characters", self.locale),
            });
        }
        if !self
            .locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!(
                    "locale '{}' contains invalid characters (allowed: A-Z, a-z, 0-9, _, -)",
                    self.locale
                ),
            });
        }
        if let Some(ref c) = self.country
            && (c.len() != 2 || !c.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!("country '{}' must be exactly two alphabetic characters", c),
            });
        }
        if let Some(ref l) = self.default_language
            && (l.len() != 2 || !l.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            return Err(Error::InvalidSection {
                section: "home",
                reason: format!(
                    "default_language '{}' must be exactly two alphabetic characters",
                    l
                ),
            });
        }
        Ok(())
    }
}

/// Split a locale tag on `_` (POSIX) or `-` (BCP-47).
/// Returns `(head, tail)` when a separator is present, otherwise `None`.
fn split_locale(locale: &str) -> Option<(&str, &str)> {
    locale.split_once('_').or_else(|| locale.split_once('-'))
}

fn locale_segment(locale: &str, index: usize) -> Option<&str> {
    locale
        .split(['_', '-'])
        .nth(index)
        .filter(|segment| !segment.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_fields_omitted() {
        let cfg: HomeConfig = toml::from_str(
            r#"
name = "test"
latitude = 0.0
longitude = 0.0
timezone = "UTC"
"#,
        )
        .unwrap();
        assert_eq!(cfg.locale, "en_US");
        assert!(cfg.units.is_none());
        assert!(cfg.country.is_none());
        assert!(cfg.default_language.is_none());
    }

    #[test]
    fn explicit_fields_parse() {
        let cfg: HomeConfig = toml::from_str(
            r#"
name = "test"
latitude = 0.0
longitude = 0.0
timezone = "UTC"
locale = "da_DK"
units = "metric"
country = "DK"
default_language = "da"
"#,
        )
        .unwrap();
        assert_eq!(cfg.locale, "da_DK");
        assert_eq!(cfg.units, Some(Units::Metric));
        assert_eq!(cfg.country, Some("DK".into()));
        assert_eq!(cfg.default_language, Some("da".into()));
    }

    #[test]
    fn resolved_units_explicit_metric() {
        let mut cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en_US".into(),
            units: Some(Units::Metric),
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Metric);
        cfg.locale = "da_DK".into();
        assert_eq!(cfg.resolved_units(), Units::Metric);
    }

    #[test]
    fn resolved_units_explicit_imperial() {
        let mut cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: Some(Units::Imperial),
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Imperial);
        cfg.locale = "en_US".into();
        assert_eq!(cfg.resolved_units(), Units::Imperial);
    }

    #[test]
    fn resolved_units_us_locale_defaults_imperial() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en_US".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Imperial);
    }

    #[test]
    fn resolved_units_us_bcp47_locale_defaults_imperial() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en-US".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Imperial);
    }

    #[test]
    fn resolved_units_case_insensitive_locale_matching() {
        for locale in ["EN_US", "En_Us", "en_us", "EN-US", "En-Us", "en-us"] {
            let cfg = HomeConfig {
                name: "t".into(),
                latitude: 0.0,
                longitude: 0.0,
                timezone: "UTC".into(),
                locale: locale.into(),
                units: None,
                country: None,
                default_language: None,
            };
            assert_eq!(
                cfg.resolved_units(),
                Units::Imperial,
                "locale {locale} should map to Imperial"
            );
        }
    }

    #[test]
    fn resolved_units_non_us_locale_defaults_metric() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Metric);
    }

    #[test]
    fn resolved_units_us_country_override_defaults_imperial() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: Some("us".into()),
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Imperial);
    }

    #[test]
    fn resolved_units_non_english_us_locale_defaults_imperial() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "es_US".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Imperial);
    }

    #[test]
    fn resolved_units_bare_language_tag_defaults_metric() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_units(), Units::Metric);
    }

    #[test]
    fn resolved_country_explicit_wins() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en_US".into(),
            units: None,
            country: Some("DK".into()),
            default_language: None,
        };
        assert_eq!(cfg.resolved_country(), Some("DK".into()));
    }

    #[test]
    fn resolved_country_derived_from_locale_underscore() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_country(), Some("DK".into()));
    }

    #[test]
    fn resolved_country_derived_from_locale_hyphen() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da-DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_country(), Some("DK".into()));
    }

    #[test]
    fn resolved_country_derived_from_locale_with_variant_suffix() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "en_US_POSIX".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_country(), Some("US".into()));
    }

    #[test]
    fn resolved_country_none_for_bare_language_tag() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_country(), None);
    }

    #[test]
    fn resolved_language_explicit_wins() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: Some("en".into()),
        };
        assert_eq!(cfg.resolved_language(), "en");
    }

    #[test]
    fn resolved_language_derived_from_locale_underscore() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_language(), "da");
    }

    #[test]
    fn resolved_language_derived_from_locale_hyphen() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da-DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_language(), "da");
    }

    #[test]
    fn resolved_language_derived_from_bare_tag() {
        let cfg = HomeConfig {
            name: "t".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert_eq!(cfg.resolved_language(), "da");
    }

    #[test]
    fn validates_valid_config() {
        let cfg = HomeConfig {
            name: "Test Home".into(),
            latitude: 56.0,
            longitude: 10.0,
            timezone: "Europe/Copenhagen".into(),
            locale: "da_DK".into(),
            units: None,
            country: Some("DK".into()),
            default_language: Some("da".into()),
        };
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_empty_locale() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_invalid_timezone() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "Not/A/Real/Timezone".into(),
            locale: "en_US".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_locale_too_long() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "a_very_long_locale_tag".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_locale_with_invalid_chars() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da@DK".into(),
            units: None,
            country: None,
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_country_not_two_alpha() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: Some("DNK".into()),
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_country_with_numbers() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: Some("D2".into()),
            default_language: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_language_not_two_alpha() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: Some("dan".into()),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_language_with_numbers() {
        let cfg = HomeConfig {
            name: "Test".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "UTC".into(),
            locale: "da_DK".into(),
            units: None,
            country: None,
            default_language: Some("d2".into()),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn old_config_without_locale_fields_validates() {
        let cfg: HomeConfig = toml::from_str(
            r#"
name = "My Home"
latitude = 37.7749
longitude = -122.4194
timezone = "America/Los_Angeles"
"#,
        )
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.locale, "en_US");
        assert_eq!(cfg.resolved_units(), Units::Imperial);
        assert_eq!(cfg.resolved_country(), Some("US".into()));
        assert_eq!(cfg.resolved_language(), "en");
    }

    #[test]
    fn units_deserializes_lowercase() {
        #[derive(Deserialize)]
        struct Wrapper {
            u: Units,
        }
        let m: Wrapper = toml::from_str("u = \"metric\"").unwrap();
        let i: Wrapper = toml::from_str("u = \"imperial\"").unwrap();
        assert_eq!(m.u, Units::Metric);
        assert_eq!(i.u, Units::Imperial);
    }
}

//! niles-config — TOML loading and validation for the Niles service.
//!
//! Per-subsystem configs (`HomeConfig`, `LightingConfig`, etc.) each
//! own their own schema and validation. The top-level `Config` is a
//! container that delegates `validate()` to each subsystem.
//!
//! Currently covered sections: `[home]`, `[lighting]`. New sections
//! land alongside the crates that consume them.

pub mod api;
pub mod error;
pub mod home;
pub mod lighting;
pub mod mqtt;
pub mod stt;
pub mod tts;
pub mod wyoming;

use serde::Deserialize;
use std::path::Path;

pub use api::ApiConfig;
pub use error::{Error, Result};
pub use home::HomeConfig;
pub use lighting::{ColorTempAnchor, LightingConfig};
pub use mqtt::MqttConfig;
pub use stt::SttConfig;
pub use tts::TtsConfig;
pub use wyoming::WyomingConfig;

/// Top-level Niles configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub home: HomeConfig,
    pub mqtt: MqttConfig,
    pub api: ApiConfig,
    pub wyoming: WyomingConfig,
    pub stt: SttConfig,
    pub tts: TtsConfig,
    pub lighting: LightingConfig,
}

impl Config {
    /// Parse from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(s)?;
        Ok(cfg)
    }

    /// Read and parse a TOML file. The path is preserved in any I/O
    /// error so `file not found` surfaces *which* file was missing.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::load_from_str(&content)
    }

    /// Verify every subsection is loadable and consistent. Call this
    /// at startup to fail-fast on a malformed config.
    ///
    /// After a successful `validate()`, call section-specific
    /// converters to obtain typed configs:
    ///
    /// ```ignore
    /// let cfg = niles_config::Config::load_from_path(path)?;
    /// cfg.validate()?;
    /// let curve = cfg.lighting.to_curve_config()
    ///     .expect("already validated above");
    /// ```
    ///
    /// The two-step (`validate()` + `to_curve_config()`) is deliberate
    /// while there's only one typed section. Once more sections exist,
    /// a future `into_validated()` will return them bundled.
    pub fn validate(&self) -> Result<()> {
        self.home.validate()?;
        self.mqtt.validate()?;
        self.api.validate()?;
        self.wyoming.validate()?;
        self.stt.validate()?;
        self.tts.validate()?;
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

[mqtt]
host = "192.168.42.16"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

[wyoming]
bind_address = "0.0.0.0:10300"

[stt]
api_key_env = "GROQ_API_KEY"

[tts]

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
    fn mqtt_defaults_are_filled_in() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert_eq!(cfg.mqtt.client_id, "niles");
        assert_eq!(cfg.mqtt.z2m_prefix, "zigbee2mqtt");
    }

    #[test]
    fn rejects_mqtt_empty_host() {
        let bad = valid_toml().replace("host = \"192.168.42.16\"", "host = \"\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_mqtt_zero_port() {
        let bad = valid_toml().replace("port = 1883", "port = 0");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_mqtt_z2m_prefix_with_wildcards() {
        let bad = valid_toml().replace(
            "username_env = \"NILES_MQTT_USERNAME\"",
            "username_env = \"NILES_MQTT_USERNAME\"\nz2m_prefix = \"zigbee2mqtt/#\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolve_credentials_reads_env_vars() {
        // Use unique var names so this test doesn't fight with parallel tests.
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::set_var("NILES_TEST_MQTT_USER", "u");
            std::env::set_var("NILES_TEST_MQTT_PASS", "p");
        }
        let cfg = MqttConfig {
            host: "h".into(),
            port: 1883,
            username_env: "NILES_TEST_MQTT_USER".into(),
            password_env: "NILES_TEST_MQTT_PASS".into(),
            client_id: "niles".into(),
            z2m_prefix: "zigbee2mqtt".into(),
        };
        let (u, p) = cfg.resolve_credentials().unwrap();
        assert_eq!(u, "u");
        assert_eq!(p, "p");
    }

    #[test]
    fn resolve_credentials_errors_when_env_var_missing() {
        let cfg = MqttConfig {
            host: "h".into(),
            port: 1883,
            username_env: "NILES_TEST_DEFINITELY_NOT_SET_USER_XYZ".into(),
            password_env: "NILES_TEST_DEFINITELY_NOT_SET_PASS_XYZ".into(),
            client_id: "niles".into(),
            z2m_prefix: "zigbee2mqtt".into(),
        };
        let err = cfg.resolve_credentials().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "mqtt",
                ..
            }
        ));
    }

    #[test]
    fn wyoming_section_parses_and_resolves_socket_addr() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        let addr = cfg.wyoming.socket_addr().unwrap();
        assert_eq!(addr.port(), 10300);
    }

    #[test]
    fn rejects_invalid_wyoming_bind_address() {
        let bad = valid_toml().replace(
            "bind_address = \"0.0.0.0:10300\"",
            "bind_address = \"not-an-address\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn stt_section_parses_with_defaults() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert_eq!(cfg.stt.api_key_env, "GROQ_API_KEY");
        assert_eq!(cfg.stt.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(cfg.stt.model, "whisper-large-v3-turbo");
        assert!(cfg.stt.language.is_none());
        assert_eq!(cfg.stt.timeout_seconds, 30);
    }

    #[test]
    fn rejects_empty_stt_api_key_env() {
        let bad = valid_toml().replace("api_key_env = \"GROQ_API_KEY\"", "api_key_env = \"\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_stt_timeout() {
        let bad = valid_toml().replace(
            "api_key_env = \"GROQ_API_KEY\"",
            "api_key_env = \"GROQ_API_KEY\"\ntimeout_seconds = 0",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_stt_language_when_set() {
        let bad = valid_toml().replace(
            "api_key_env = \"GROQ_API_KEY\"",
            "api_key_env = \"GROQ_API_KEY\"\nlanguage = \"\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_stt_base_url_without_http_scheme() {
        let bad = valid_toml().replace(
            "api_key_env = \"GROQ_API_KEY\"",
            "api_key_env = \"GROQ_API_KEY\"\nbase_url = \"api.groq.com/openai/v1\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolve_stt_api_key_reads_env_var() {
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::set_var("NILES_TEST_GROQ_API_KEY", "gsk_test");
        }
        let cfg = SttConfig {
            api_key_env: "NILES_TEST_GROQ_API_KEY".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            language: None,
            timeout_seconds: 30,
        };
        assert_eq!(cfg.resolve_api_key().unwrap(), "gsk_test");
    }

    #[test]
    fn resolve_stt_api_key_errors_when_missing() {
        // Guarantee the var is unset even if a parallel test, an
        // ambient shell export, or a prior iteration of this test
        // body set it.
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::remove_var("NILES_TEST_DEFINITELY_NOT_SET_STT_KEY_XYZ");
        }
        let cfg = SttConfig {
            api_key_env: "NILES_TEST_DEFINITELY_NOT_SET_STT_KEY_XYZ".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            language: None,
            timeout_seconds: 30,
        };
        let err = cfg.resolve_api_key().unwrap_err();
        assert!(matches!(err, Error::InvalidSection { section: "stt", .. }));
    }

    #[test]
    fn api_section_parses_and_resolves_socket_addr() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        let addr = cfg.api.socket_addr().unwrap();
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn rejects_invalid_api_bind_address() {
        let bad = valid_toml().replace(
            "bind_address = \"0.0.0.0:8080\"",
            "bind_address = \"not-an-address\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn load_from_path_includes_path_in_error() {
        let missing = std::path::Path::new("does/not/exist/niles.toml");
        let err = Config::load_from_path(missing).unwrap_err();
        match err {
            Error::Read { path, .. } => {
                assert_eq!(path, missing, "Error::Read should carry the offending path");
                // Display impl should mention the path so it's visible in logs.
                let rendered = format!(
                    "{}",
                    Error::Read {
                        path: path.clone(),
                        source: std::io::Error::other("test"),
                    }
                );
                assert!(
                    rendered.contains("does"),
                    "expected path in rendered error, got: {rendered}"
                );
            }
            other => panic!("expected Error::Read, got {other:?}"),
        }
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

    #[test]
    fn tts_section_parses_with_defaults() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert_eq!(
            cfg.tts.base_url,
            "http://piper.home-automation.svc.cluster.local:5000"
        );
        assert_eq!(cfg.tts.default_voice, "en_GB-alan-medium");
        assert_eq!(cfg.tts.timeout_seconds, 30);
    }

    #[test]
    fn rejects_empty_tts_default_voice() {
        let bad = valid_toml().replace("[tts]", "[tts]\ndefault_voice = \"\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_tts_base_url_without_http_scheme() {
        let bad = valid_toml().replace("[tts]", "[tts]\nbase_url = \"piper.local:5000\"");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_tts_timeout() {
        let bad = valid_toml().replace("[tts]", "[tts]\ntimeout_seconds = 0");
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }
}

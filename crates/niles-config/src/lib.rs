//! niles-config — TOML loading and validation for the Niles service.
//!
//! Per-subsystem configs (`HomeConfig`, `LightingConfig`, etc.) each
//! own their own schema and validation. The top-level `Config` is a
//! container that delegates `validate()` to each subsystem. New
//! sections land alongside the crates that consume them.

pub mod ambient_lights;
pub mod api;
pub mod capabilities;
pub mod error;
pub mod history;
pub mod home;
pub mod lighting;
pub mod llm;
pub mod memory;
pub mod mqtt;
pub mod notifications;
pub mod persistence;
pub mod presence;
pub mod recognition;
pub mod satellites;
pub mod skills;
pub mod speakers;
pub mod stt;
pub mod tts;
pub mod web_search;
pub mod wyoming;

use serde::Deserialize;
use std::path::Path;

pub use ambient_lights::AmbientLightsConfig;
pub use api::ApiConfig;
pub use capabilities::CapabilitiesConfig;
pub use error::{Error, Result};
pub use history::HistoryConfig;
pub use home::{HomeConfig, Units};
pub use lighting::{ColorTempAnchor, LightingConfig, MorningRoutineConfigDto};
pub use llm::{LlmConfig, LlmTier2Config};
pub use memory::MemoryConfig;
pub use mqtt::MqttConfig;
pub use notifications::NotificationsConfig;
pub use persistence::PersistenceConfig;
pub use presence::{PresenceConfig, TadoConfigDto};
pub use recognition::{MatchStrategy, MatcherConfig, RecognitionConfig};
pub use satellites::{SatelliteConfig, SatellitesConfig};
pub use skills::{SkillsConfig, SkillsCuratorConfig, SkillsReviewConfig};
pub use speakers::{SpeakerConfig, SpeakersConfig};
pub use stt::SttConfig;
pub use tts::TtsConfig;
pub use web_search::WebSearchConfig;
pub use wyoming::WyomingConfig;

/// Top-level Niles configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub home: HomeConfig,
    pub mqtt: MqttConfig,
    pub api: ApiConfig,
    #[serde(default)]
    pub capabilities: CapabilitiesConfig,
    #[serde(default)]
    pub persistence: PersistenceConfig,
    #[serde(default)]
    pub recognition: RecognitionConfig,
    #[serde(default)]
    pub satellites: SatellitesConfig,
    #[serde(default)]
    pub speakers: SpeakersConfig,
    #[serde(default)]
    pub ambient_lights: AmbientLightsConfig,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub presence: PresenceConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub web_search: WebSearchConfig,
    pub wyoming: WyomingConfig,
    pub stt: SttConfig,
    pub tts: TtsConfig,
    pub llm: LlmConfig,
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
        self.capabilities.validate()?;
        self.persistence.validate()?;
        self.recognition.validate()?;
        self.satellites.validate()?;
        self.speakers.validate()?;
        self.ambient_lights.validate()?;
        self.history.validate()?;
        self.memory.validate()?;
        self.notifications.validate()?;
        self.presence.validate()?;
        self.skills.validate()?;
        self.web_search.validate()?;
        self.wyoming.validate()?;
        self.stt.validate()?;
        self.tts.validate()?;
        self.llm.validate()?;
        let _ = self.lighting.to_curve_config()?;
        if let Some(routine) = &self.lighting.morning_routine {
            let _ = routine.to_morning_routine_config()?;
        }
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

[llm]
api_key_env = "GROQ_API_KEY"

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
    fn ambient_lights_section_parses_and_validates() {
        let toml = format!(
            "{}\n[ambient_lights]\ndevices = [\"living_room/tv_lightstrip\"]\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.ambient_lights.devices.len(), 1);
        assert_eq!(cfg.ambient_lights.devices[0], "living_room/tv_lightstrip");
    }

    #[test]
    fn ambient_lights_invalid_device_id_surfaces_section_name() {
        let toml = format!(
            "{}\n[ambient_lights]\ndevices = [\"not_a_valid_id\"]\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("ambient_lights"),
            "error should mention section name: {msg}"
        );
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
        let bad = valid_toml().replace(
            "[stt]\napi_key_env = \"GROQ_API_KEY\"",
            "[stt]\napi_key_env = \"\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_stt_timeout() {
        let bad = valid_toml().replace(
            "[stt]\napi_key_env = \"GROQ_API_KEY\"",
            "[stt]\napi_key_env = \"GROQ_API_KEY\"\ntimeout_seconds = 0",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_stt_language_when_set() {
        let bad = valid_toml().replace(
            "[stt]\napi_key_env = \"GROQ_API_KEY\"",
            "[stt]\napi_key_env = \"GROQ_API_KEY\"\nlanguage = \"\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_stt_base_url_without_http_scheme() {
        let bad = valid_toml().replace(
            "[stt]\napi_key_env = \"GROQ_API_KEY\"",
            "[stt]\napi_key_env = \"GROQ_API_KEY\"\nbase_url = \"api.groq.com/openai/v1\"",
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

    // ------------------------------------------------------------------
    // LLM section tests
    // ------------------------------------------------------------------

    #[test]
    fn llm_section_parses_with_defaults() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert_eq!(cfg.llm.api_key_env, "GROQ_API_KEY");
        assert_eq!(cfg.llm.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(cfg.llm.model, "openai/gpt-oss-20b");
        assert_eq!(cfg.llm.timeout_seconds, 30);
    }

    #[test]
    fn rejects_empty_llm_api_key_env() {
        let bad = valid_toml().replace(
            "[llm]\napi_key_env = \"GROQ_API_KEY\"",
            "[llm]\napi_key_env = \"\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_llm_timeout() {
        let bad = valid_toml().replace(
            "[llm]\napi_key_env = \"GROQ_API_KEY\"",
            "[llm]\napi_key_env = \"GROQ_API_KEY\"\ntimeout_seconds = 0",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_llm_base_url_without_http_scheme() {
        let bad = valid_toml().replace(
            "[llm]\napi_key_env = \"GROQ_API_KEY\"",
            "[llm]\napi_key_env = \"GROQ_API_KEY\"\nbase_url = \"api.groq.com/openai/v1\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_llm_model() {
        let bad = valid_toml().replace(
            "[llm]\napi_key_env = \"GROQ_API_KEY\"",
            "[llm]\napi_key_env = \"GROQ_API_KEY\"\nmodel = \"\"",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolve_llm_api_key_reads_env_var() {
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::set_var("NILES_TEST_LLM_GROQ_API_KEY", "gsk_test_llm");
        }
        let cfg = LlmConfig {
            api_key_env: "NILES_TEST_LLM_GROQ_API_KEY".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            timeout_seconds: 30,
            tier2: None,
        };
        assert_eq!(cfg.resolve_api_key().unwrap(), "gsk_test_llm");
    }

    #[test]
    fn resolve_llm_api_key_errors_when_missing() {
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::remove_var("NILES_TEST_DEFINITELY_NOT_SET_LLM_KEY_XYZ");
        }
        let cfg = LlmConfig {
            api_key_env: "NILES_TEST_DEFINITELY_NOT_SET_LLM_KEY_XYZ".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            timeout_seconds: 30,
            tier2: None,
        };
        let err = cfg.resolve_api_key().unwrap_err();
        assert!(matches!(err, Error::InvalidSection { section: "llm", .. }));
    }

    #[test]
    fn llm_tier2_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert!(cfg.llm.tier2.is_none());
    }

    #[test]
    fn llm_tier2_parses_with_defaults() {
        let toml = format!(
            "{}\n[llm.tier2]\napi_key_env = \"OPENAI_API_KEY\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let tier2 = cfg.llm.tier2.as_ref().unwrap();
        assert_eq!(tier2.api_key_env, "OPENAI_API_KEY");
        assert_eq!(tier2.base_url, "https://api.openai.com/v1");
        assert_eq!(tier2.model, "gpt-5.5");
        assert_eq!(tier2.timeout_seconds, 30);
    }

    #[test]
    fn llm_tier2_explicit_overrides_parse() {
        let toml = format!(
            r#"{}
[llm.tier2]
api_key_env = "OPENAI_API_KEY"
base_url = "https://custom.openai.com/v1"
model = "gpt-5.5-pro"
timeout_seconds = 60
"#,
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let tier2 = cfg.llm.tier2.as_ref().unwrap();
        assert_eq!(tier2.base_url, "https://custom.openai.com/v1");
        assert_eq!(tier2.model, "gpt-5.5-pro");
        assert_eq!(tier2.timeout_seconds, 60);
    }

    #[test]
    fn rejects_empty_llm_tier2_api_key_env() {
        let toml = format!(
            "{}\n[llm.tier2]\napi_key_env = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_llm_tier2_timeout() {
        let toml = format!(
            "{}\n[llm.tier2]\napi_key_env = \"OPENAI_API_KEY\"\ntimeout_seconds = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_llm_tier2_base_url_without_http_scheme() {
        let toml = format!(
            "{}\n[llm.tier2]\napi_key_env = \"OPENAI_API_KEY\"\nbase_url = \"api.openai.com/v1\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_empty_llm_tier2_model() {
        let toml = format!(
            "{}\n[llm.tier2]\napi_key_env = \"OPENAI_API_KEY\"\nmodel = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolve_llm_tier2_api_key_reads_env_var() {
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::set_var("NILES_TEST_TIER2_KEY", "sk_test_tier2");
        }
        let cfg = LlmTier2Config {
            api_key_env: "NILES_TEST_TIER2_KEY".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            timeout_seconds: 30,
        };
        assert_eq!(cfg.resolve_api_key().unwrap(), "sk_test_tier2");
    }

    #[test]
    fn resolve_llm_tier2_api_key_errors_when_missing() {
        // SAFETY: in #[cfg(test)] only.
        unsafe {
            std::env::remove_var("NILES_TEST_DEFINITELY_NOT_SET_TIER2_KEY_XYZ");
        }
        let cfg = LlmTier2Config {
            api_key_env: "NILES_TEST_DEFINITELY_NOT_SET_TIER2_KEY_XYZ".into(),
            base_url: "https://example".into(),
            model: "m".into(),
            timeout_seconds: 30,
        };
        let err = cfg.resolve_api_key().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "llm.tier2",
                ..
            }
        ));
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
    fn home_locale_fields_explicit_in_full_config() {
        let toml = valid_toml().trim_end_matches('\n').replace(
            "[home]\nname = \"test home\"",
            r#"[home]
name = "test home"
locale = "da_DK"
units = "metric"
country = "DK"
default_language = "da""#,
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.home.locale, "da_DK");
        assert_eq!(cfg.home.units, Some(Units::Metric));
        assert_eq!(cfg.home.country, Some("DK".into()));
        assert_eq!(cfg.home.default_language, Some("da".into()));
        assert_eq!(cfg.home.resolved_units(), Units::Metric);
        assert_eq!(cfg.home.resolved_country(), Some("DK".into()));
        assert_eq!(cfg.home.resolved_language(), "da");
    }

    #[test]
    fn home_locale_defaults_in_full_config() {
        // valid_toml() omits locale fields — they should default correctly.
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.home.locale, "en_US");
        assert_eq!(cfg.home.units, None);
        assert_eq!(cfg.home.country, None);
        assert_eq!(cfg.home.default_language, None);
        assert_eq!(cfg.home.resolved_units(), Units::Imperial);
        assert_eq!(cfg.home.resolved_country(), Some("US".into()));
        assert_eq!(cfg.home.resolved_language(), "en");
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

    // ------------------------------------------------------------------
    // TTS section tests (from main / PR #29)
    // ------------------------------------------------------------------

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

    // ------------------------------------------------------------------
    // Morning routine config tests
    // ------------------------------------------------------------------

    #[test]
    fn routine_absent_when_omitted() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        assert!(cfg.lighting.morning_routine.is_none());
    }

    fn valid_toml_with_routine() -> String {
        format!(
            r#"{}
[lighting.morning_routine]
fire_days = ["mon", "tue", "wed", "thu", "fri"]
target_devices = ["z2m:bedroom/ceiling_light", "z2m:hallway/main"]
skip_overrides = ["2026-12-25", "2026-12-31"]
"#,
            valid_toml()
        )
    }

    #[test]
    fn routine_section_round_trips() {
        let cfg = Config::load_from_str(&valid_toml_with_routine()).unwrap();
        cfg.validate().unwrap();
        let routine = cfg.lighting.morning_routine.as_ref().unwrap();
        let typed = routine.to_morning_routine_config().unwrap();
        assert_eq!(typed.fire_days.len(), 5);
        assert_eq!(typed.target_devices.len(), 2);
        assert_eq!(typed.skip_overrides.len(), 2);
        assert_eq!(
            typed.target_devices[0].to_string(),
            "z2m:bedroom/ceiling_light"
        );
    }

    #[test]
    fn rejects_bad_weekday() {
        let bad = valid_toml_with_routine().replace(
            "fire_days = [\"mon\", \"tue\", \"wed\", \"thu\", \"fri\"]",
            "fire_days = [\"funday\"]",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("funday"),
            "error should contain offending input: {msg}"
        );
    }

    #[test]
    fn rejects_bad_device_id() {
        let bad = valid_toml_with_routine().replace(
            "target_devices = [\"z2m:bedroom/ceiling_light\", \"z2m:hallway/main\"]",
            "target_devices = [\"not_a_device_id\"]",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not_a_device_id"),
            "error should contain offending input: {msg}"
        );
    }

    #[test]
    fn rejects_bad_skip_date() {
        let bad = valid_toml_with_routine().replace(
            "skip_overrides = [\"2026-12-25\", \"2026-12-31\"]",
            "skip_overrides = [\"2026-13-99\"]",
        );
        let cfg = Config::load_from_str(&bad).unwrap();
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("2026-13-99"),
            "error should contain offending input: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_routine_field() {
        let bad = valid_toml_with_routine().replace(
            "[lighting.morning_routine]",
            "[lighting.morning_routine]\nunknown_field = 42",
        );
        assert!(Config::load_from_str(&bad).is_err());
    }

    // ------------------------------------------------------------------
    // Capabilities section tests
    // ------------------------------------------------------------------

    #[test]
    fn capabilities_section_absent_defaults_to_none() {
        // The existing valid_toml() has no [capabilities] section.
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.capabilities.directory.is_none());
    }

    #[test]
    fn capabilities_directory_parses_into_pathbuf() {
        let toml = format!(
            "{}\n[capabilities]\ndirectory = \"/x\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.capabilities.directory.as_deref(),
            Some(std::path::Path::new("/x"))
        );
    }

    #[test]
    fn rejects_empty_capabilities_directory() {
        let toml = format!(
            "{}\n[capabilities]\ndirectory = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "capabilities",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Persistence section tests
    // ------------------------------------------------------------------

    #[test]
    fn persistence_section_absent_defaults_to_none() {
        // The existing valid_toml() has no [persistence] section.
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.persistence.directory.is_none());
    }

    #[test]
    fn persistence_directory_parses_into_pathbuf() {
        let toml = format!(
            "{}\n[persistence]\ndirectory = \"/var/niles\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.persistence.directory.as_deref(),
            Some(std::path::Path::new("/var/niles"))
        );
    }

    #[test]
    fn rejects_empty_persistence_directory() {
        let toml = format!(
            "{}\n[persistence]\ndirectory = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "persistence",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // History section tests
    // ------------------------------------------------------------------

    #[test]
    fn history_section_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.history.directory.is_none());
        assert_eq!(cfg.history.retention_days, 14);
    }

    #[test]
    fn history_directory_parses_into_pathbuf() {
        let toml = format!(
            "{}\n[history]\ndirectory = \"/var/niles/history\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.history.directory.as_deref(),
            Some(std::path::Path::new("/var/niles/history"))
        );
        assert_eq!(cfg.history.retention_days, 14);
    }

    #[test]
    fn history_retention_days_explicit() {
        let toml = format!(
            "{}\n[history]\ndirectory = \"/tmp\"\nretention_days = 30\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.history.retention_days, 30);
    }

    #[test]
    fn rejects_empty_history_directory() {
        let toml = format!(
            "{}\n[history]\ndirectory = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "history",
                ..
            }
        ));
    }

    #[test]
    fn rejects_history_retention_days_zero() {
        let toml = format!(
            "{}\n[history]\ndirectory = \"/tmp\"\nretention_days = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "history",
                ..
            }
        ));
    }

    #[test]
    fn rejects_history_retention_days_too_large() {
        let toml = format!(
            "{}\n[history]\ndirectory = \"/tmp\"\nretention_days = 366\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "history",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Memory section tests
    // ------------------------------------------------------------------

    #[test]
    fn memory_section_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.memory.directory.is_none());
        assert_eq!(cfg.memory.user_char_limit, 1375);
        assert_eq!(cfg.memory.agent_char_limit, 2200);
    }

    #[test]
    fn memory_directory_parses_into_pathbuf() {
        let toml = format!(
            "{}\n[memory]\ndirectory = \"/var/niles/memory\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.memory.directory.as_deref(),
            Some(std::path::Path::new("/var/niles/memory"))
        );
        assert_eq!(cfg.memory.user_char_limit, 1375);
        assert_eq!(cfg.memory.agent_char_limit, 2200);
    }

    #[test]
    fn memory_char_limits_explicit() {
        let toml = format!(
            "{}\n[memory]\ndirectory = \"/tmp\"\nuser_char_limit = 500\nagent_char_limit = 1000\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.memory.user_char_limit, 500);
        assert_eq!(cfg.memory.agent_char_limit, 1000);
    }

    #[test]
    fn rejects_empty_memory_directory() {
        let toml = format!(
            "{}\n[memory]\ndirectory = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "memory",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_user_char_limit() {
        let toml = format!(
            "{}\n[memory]\ndirectory = \"/tmp\"\nuser_char_limit = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "memory",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_agent_char_limit() {
        let toml = format!(
            "{}\n[memory]\ndirectory = \"/tmp\"\nagent_char_limit = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "memory",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Skills section tests
    // ------------------------------------------------------------------

    #[test]
    fn skills_section_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.skills.directory.is_none());
        assert_eq!(cfg.skills.skill_max_chars, 100_000);
        assert_eq!(cfg.skills.supporting_file_max_bytes, 1_048_576);
    }

    #[test]
    fn skills_directory_parses_into_pathbuf() {
        let toml = format!(
            "{}\n[skills]\ndirectory = \"/var/niles/skills\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.skills.directory.as_deref(),
            Some(std::path::Path::new("/var/niles/skills"))
        );
        assert_eq!(cfg.skills.skill_max_chars, 100_000);
        assert_eq!(cfg.skills.supporting_file_max_bytes, 1_048_576);
    }

    #[test]
    fn skills_limits_explicit() {
        let toml = format!(
            "{}\n[skills]\ndirectory = \"/tmp\"\nskill_max_chars = 50_000\nsupporting_file_max_bytes = 524_288\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.skills.skill_max_chars, 50_000);
        assert_eq!(cfg.skills.supporting_file_max_bytes, 524_288);
    }

    #[test]
    fn rejects_empty_skills_directory() {
        let toml = format!(
            "{}\n[skills]\ndirectory = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_skill_max_chars() {
        let toml = format!(
            "{}\n[skills]\ndirectory = \"/tmp\"\nskill_max_chars = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_supporting_file_max_bytes() {
        let toml = format!(
            "{}\n[skills]\ndirectory = \"/tmp\"\nsupporting_file_max_bytes = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn skills_curator_defaults_when_omitted() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.skills.curator.enabled);
        assert_eq!(cfg.skills.curator.interval_hours, 24);
        assert_eq!(cfg.skills.curator.stale_after_days, 30);
        assert_eq!(cfg.skills.curator.archive_after_days, 90);
    }

    #[test]
    fn skills_curator_explicit_overrides_parse() {
        let toml = format!(
            "{}\n[skills.curator]\nenabled = false\ninterval_hours = 12\nstale_after_days = 15\narchive_after_days = 45\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.skills.curator.enabled);
        assert_eq!(cfg.skills.curator.interval_hours, 12);
        assert_eq!(cfg.skills.curator.stale_after_days, 15);
        assert_eq!(cfg.skills.curator.archive_after_days, 45);
    }

    #[test]
    fn skills_curator_disabled() {
        let toml = format!(
            "{}\n[skills.curator]\nenabled = false\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.skills.curator.enabled);
    }

    #[test]
    fn rejects_archive_after_less_than_stale_after() {
        let toml = format!(
            "{}\n[skills.curator]\narchive_after_days = 10\nstale_after_days = 20\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_stale_after_days() {
        let toml = format!(
            "{}\n[skills.curator]\nstale_after_days = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_interval_hours() {
        let toml = format!(
            "{}\n[skills.curator]\ninterval_hours = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_large_interval_hours() {
        let toml = format!(
            "{}\n[skills.curator]\ninterval_hours = 9000\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_curator_field() {
        let toml = format!(
            "{}\n[skills.curator]\nunknown_field = 42\n",
            valid_toml().trim_end_matches('\n')
        );
        assert!(Config::load_from_str(&toml).is_err());
    }

    #[test]
    fn rejects_too_large_stale_after_days() {
        let toml = format!(
            "{}\n[skills.curator]\nstale_after_days = 4000\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_large_archive_after_days() {
        let toml = format!(
            "{}\n[skills.curator]\narchive_after_days = 4000\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "skills",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Web search section tests
    // ------------------------------------------------------------------

    #[test]
    fn web_search_section_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.web_search.base_url.is_none());
        assert_eq!(cfg.web_search.timeout_seconds, 15);
        assert_eq!(cfg.web_search.default_num_results, 5);
    }

    #[test]
    fn web_search_base_url_parses() {
        let toml = format!(
            "{}\n[web_search]\nbase_url = \"https://search.mnygaard.io/search\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.web_search.base_url,
            Some("https://search.mnygaard.io/search".into())
        );
    }

    #[test]
    fn rejects_empty_web_search_base_url() {
        let toml = format!(
            "{}\n[web_search]\nbase_url = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn rejects_web_search_base_url_without_http_scheme() {
        let toml = format!(
            "{}\n[web_search]\nbase_url = \"search.example.com\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_web_search_timeout() {
        let toml = format!(
            "{}\n[web_search]\ntimeout_seconds = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_large_web_search_timeout() {
        let toml = format!(
            "{}\n[web_search]\ntimeout_seconds = 601\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_web_search_num_results() {
        let toml = format!(
            "{}\n[web_search]\ndefault_num_results = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_large_web_search_num_results() {
        let toml = format!(
            "{}\n[web_search]\ndefault_num_results = 50\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "web_search",
                ..
            }
        ));
    }

    #[test]
    fn web_search_section_explicit_overrides_parse() {
        let toml = format!(
            "{}\n[web_search]\nbase_url = \"https://search.example.com/search\"\ntimeout_seconds = 30\ndefault_num_results = 10\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.web_search.base_url,
            Some("https://search.example.com/search".into())
        );
        assert_eq!(cfg.web_search.timeout_seconds, 30);
        assert_eq!(cfg.web_search.default_num_results, 10);
    }

    // ------------------------------------------------------------------
    // Recognition section tests
    // ------------------------------------------------------------------

    #[test]
    fn recognition_section_absent_defaults_to_none() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.recognition.model_path.is_none());
        assert!(!cfg.recognition.use_gpu);
    }

    #[test]
    fn recognition_model_path_parses_when_absolute() {
        let toml = format!(
            "{}\n[recognition]\nmodel_path = \"/var/niles/ecapa.onnx\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(
            cfg.recognition.model_path.as_deref(),
            Some(std::path::Path::new("/var/niles/ecapa.onnx"))
        );
    }

    #[test]
    fn recognition_use_gpu_explicit() {
        let toml = format!(
            "{}\n[recognition]\nmodel_path = \"/var/niles/ecapa.onnx\"\nuse_gpu = true\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.recognition.use_gpu);
    }

    #[test]
    fn rejects_relative_recognition_model_path() {
        let toml = format!(
            "{}\n[recognition]\nmodel_path = \"./ecapa.onnx\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "recognition",
                ..
            }
        ));
    }

    #[test]
    fn rejects_empty_recognition_model_path() {
        let toml = format!(
            "{}\n[recognition]\nmodel_path = \"\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "recognition",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Recognition matcher section tests
    // ------------------------------------------------------------------

    #[test]
    fn recognition_matcher_section_absent_defaults() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!((cfg.recognition.matcher.threshold - 0.65).abs() < 1e-6);
        assert_eq!(
            cfg.recognition.matcher.strategy,
            MatchStrategy::MaxSimilarity
        );
        assert!(cfg.recognition.matcher.enrollment_dir.is_none());
    }

    #[test]
    fn recognition_matcher_threshold_parses() {
        let toml = format!(
            "{}\n[recognition.matcher]\nthreshold = 0.8\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert!((cfg.recognition.matcher.threshold - 0.8).abs() < 1e-6);
    }

    #[test]
    fn rejects_threshold_above_one() {
        let toml = format!(
            "{}\n[recognition.matcher]\nthreshold = 1.5\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "recognition.matcher",
                ..
            }
        ));
    }

    #[test]
    fn rejects_threshold_below_zero() {
        let toml = format!(
            "{}\n[recognition.matcher]\nthreshold = -0.1\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "recognition.matcher",
                ..
            }
        ));
    }

    #[test]
    fn recognition_matcher_strategy_parses_centroid() {
        let toml = format!(
            "{}\n[recognition.matcher]\nstrategy = \"centroid\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.recognition.matcher.strategy, MatchStrategy::Centroid);
    }

    #[test]
    fn rejects_unknown_matcher_field() {
        let toml = format!(
            "{}\n[recognition.matcher]\nunknown_field = 42\n",
            valid_toml().trim_end_matches('\n')
        );
        assert!(Config::load_from_str(&toml).is_err());
    }

    #[test]
    fn rejects_relative_enrollment_dir() {
        let toml = format!(
            "{}\n[recognition.matcher]\nenrollment_dir = \"./enrolled\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidSection {
                section: "recognition.matcher",
                ..
            }
        ));
    }

    // ------------------------------------------------------------------
    // Presence section tests
    // ------------------------------------------------------------------

    #[test]
    fn presence_section_absent_defaults_to_disabled() {
        let cfg = Config::load_from_str(valid_toml()).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.presence.enabled);
        assert_eq!(cfg.presence.poll_seconds, 60);
        assert_eq!(cfg.presence.away_debounce_minutes, 5);
        assert!(cfg.presence.tado.is_none());
    }

    #[test]
    fn presence_with_tado_parses_and_validates() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\n[presence.tado]\nusername_env = \"TADO_USER\"\npassword_env = \"TADO_PASS\"\nhome_id = 123\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.presence.enabled);
        let tado = cfg.presence.tado.as_ref().unwrap();
        assert_eq!(tado.username_env, "TADO_USER");
        assert_eq!(tado.home_id, 123);
    }

    #[test]
    fn rejects_presence_enabled_without_source() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_presence_poll_seconds_too_small() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\npoll_seconds = 5\n[presence.tado]\nusername_env = \"U\"\npassword_env = \"P\"\nhome_id = 1\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_presence_poll_seconds_too_large() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\npoll_seconds = 4000\n[presence.tado]\nusername_env = \"U\"\npassword_env = \"P\"\nhome_id = 1\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_presence_away_debounce_too_large() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\naway_debounce_minutes = 121\n[presence.tado]\nusername_env = \"U\"\npassword_env = \"P\"\nhome_id = 1\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_empty_tado_username_env() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\n[presence.tado]\nusername_env = \"\"\npassword_env = \"P\"\nhome_id = 1\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_zero_tado_home_id() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\n[presence.tado]\nusername_env = \"U\"\npassword_env = \"P\"\nhome_id = 0\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_tado_base_url_without_http_scheme() {
        let toml = format!(
            "{}\n[presence]\nenabled = true\n[presence.tado]\nusername_env = \"U\"\npassword_env = \"P\"\nhome_id = 1\nbase_url = \"my.tado.com\"\n",
            valid_toml().trim_end_matches('\n')
        );
        let cfg = Config::load_from_str(&toml).unwrap();
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
    fn rejects_unknown_presence_field() {
        let toml = format!(
            "{}\n[presence]\nunknown_field = 42\n",
            valid_toml().trim_end_matches('\n')
        );
        assert!(Config::load_from_str(&toml).is_err());
    }
}

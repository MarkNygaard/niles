//! What is still unset, and what it costs.
//!
//! Every section has a default now, so Niles starts with no config file
//! at all. That is what makes the app usable as the way in — but it
//! also means *starting* no longer proves anything, and something has
//! to carry the difference between "running" and "configured".
//!
//! This is that something. It reports gaps rather than refusing to
//! start, because a Niles that will not boot cannot be told the answer.

use crate::Config;
use serde::Serialize;

/// How much a gap costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    /// Nothing works until this is set.
    Blocking,
    /// One feature is off; the rest of the house is fine.
    Degraded,
}

/// Something that has not been set yet.
#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    /// The config path, so the UI can link straight at it.
    pub path: &'static str,
    pub severity: Severity,
    /// What does not work, in the terms of somebody using the house
    /// rather than the terms of the config file.
    pub consequence: &'static str,
}

impl Config {
    /// Everything still needed, worst first.
    ///
    /// Empty means Niles is fully set up. It deliberately does not
    /// include anything with a working default — a gap is something a
    /// person has to answer, not something they *could* change.
    pub fn setup_gaps(&self) -> Vec<Gap> {
        let mut gaps = Vec::new();

        if self.mqtt.host.trim().is_empty() {
            gaps.push(Gap {
                path: "mqtt.host",
                severity: Severity::Blocking,
                consequence: "Niles cannot reach any lights: it has no broker to talk to.",
            });
        }
        if self.mqtt.username_env.trim().is_empty() {
            gaps.push(Gap {
                path: "mqtt.username_env",
                severity: Severity::Blocking,
                consequence: "The broker will refuse the connection without a username.",
            });
        }

        if self.mqtt.password_env.trim().is_empty() {
            gaps.push(Gap {
                path: "mqtt.password_env",
                severity: Severity::Blocking,
                consequence: "The broker will refuse the connection without a password.",
            });
        }

        if self.llm.api_key_env.trim().is_empty() {
            gaps.push(Gap {
                path: "llm.api_key_env",
                severity: Severity::Degraded,
                consequence: "Anything a regex cannot answer goes unanswered.",
            });
        }
        if self.stt.api_key_env.trim().is_empty() {
            gaps.push(Gap {
                path: "stt.api_key_env",
                severity: Severity::Degraded,
                consequence: "Speech cannot be transcribed, so voice does nothing.",
            });
        }

        // UTC is the default because guessing a zone would put the
        // curve an hour out and look deliberate. Left as UTC, the curve
        // runs to the wrong clock — which is worth saying out loud.
        if self.home.timezone == "UTC" {
            gaps.push(Gap {
                path: "home.timezone",
                severity: Severity::Degraded,
                consequence: "The curve and the morning routine run on UTC, not your clock.",
            });
        }
        if self.home.latitude == 0.0 && self.home.longitude == 0.0 {
            gaps.push(Gap {
                path: "home.latitude",
                severity: Severity::Degraded,
                consequence: "Weather answers are for the Gulf of Guinea.",
            });
        }

        gaps.sort_by_key(|g| match g.severity {
            Severity::Blocking => 0,
            Severity::Degraded => 1,
        });
        gaps
    }

    /// True when nothing is left to answer.
    pub fn is_set_up(&self) -> bool {
        self.setup_gaps().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_that_says_nothing_still_parses() {
        // The whole point: Niles starts with no file, so the app can be
        // the way in. A config that refuses to load cannot be told
        // anything.
        let cfg = Config::load_from_str("").expect("an empty config is a valid one");
        assert_eq!(cfg.api.bind_address, "0.0.0.0:8080");
        assert!(cfg.lighting.enabled);
        assert_eq!(cfg.lighting.color_temp_anchors.len(), 7);
    }

    #[test]
    fn the_shipped_defaults_are_a_valid_config() {
        // Defaults that do not validate would turn a first start into a
        // crash loop, which is the failure this change exists to avoid.
        Config::load_from_str("")
            .expect("parses")
            .validate()
            .expect("and is valid");
    }

    #[test]
    fn an_empty_config_says_what_it_is_missing() {
        let gaps = Config::load_from_str("").unwrap().setup_gaps();
        let paths: Vec<_> = gaps.iter().map(|g| g.path).collect();
        assert!(paths.contains(&"mqtt.host"), "{paths:?}");
        assert_eq!(
            gaps[0].severity,
            Severity::Blocking,
            "the worst gap comes first, so a page can lead with it"
        );
    }

    #[test]
    fn a_broker_that_is_set_is_not_a_gap() {
        let toml = "[mqtt]\nhost = \"192.168.42.16\"\nusername_env = \"U\"\npassword_env = \"P\"\n";
        let gaps = Config::load_from_str(toml).unwrap().setup_gaps();
        assert!(
            !gaps.iter().any(|g| g.path.starts_with("mqtt.")),
            "{gaps:?}"
        );
    }

    #[test]
    fn nothing_left_to_answer_reads_as_set_up() {
        let toml = concat!(
            "[home]\nname = \"Home\"\nlatitude = 56.1572\nlongitude = 10.2107\n",
            "timezone = \"Europe/Copenhagen\"\n",
            "[mqtt]\nhost = \"broker\"\nusername_env = \"U\"\npassword_env = \"P\"\n",
            "[stt]\napi_key_env = \"K\"\n[llm]\napi_key_env = \"K\"\n",
        );
        let cfg = Config::load_from_str(toml).unwrap();
        assert!(cfg.is_set_up(), "{:?}", cfg.setup_gaps());
    }
}

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
use crate::providers::Retired;
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
    pub consequence: String,
}

impl Config {
    /// Everything still needed, worst first.
    ///
    /// Empty means Niles is fully set up. It deliberately does not
    /// include anything with a working default — a gap is something a
    /// person has to answer, not something they *could* change.
    #[expect(
        clippy::too_many_lines,
        reason = "a flat list of independent setup checks, one block per gap"
    )]
    pub fn setup_gaps(&self) -> Vec<Gap> {
        let mut gaps = Vec::new();

        if self.mqtt.host.trim().is_empty() {
            gaps.push(Gap {
                path: "mqtt.host",
                severity: Severity::Blocking,
                consequence: "Niles can't reach any lights yet: it has no broker to talk to."
                    .into(),
            });
        }

        // Asked of the resolver, not of the config's `*_env` names.
        // Checking whether a variable is *named* answers the wrong
        // question once a credential can also be stored: a broker
        // password typed into Settings works, and reported a missing
        // one — which is the same mistake `/secrets` made, in a second
        // place, and looks worse here because it is the first thing on
        // the page.
        let label = |provider: Option<&str>| {
            provider
                .and_then(crate::catalogue::find)
                .map_or("Its endpoint", |known| known.label)
        };
        for (key, severity, consequence) in [
            (
                "mqtt.username",
                Severity::Blocking,
                "The broker will refuse the connection without a username.".to_string(),
            ),
            (
                "mqtt.password",
                Severity::Blocking,
                "The broker will refuse the connection without a password.".to_string(),
            ),
            (
                "llm.api_key",
                Severity::Degraded,
                format!(
                    "Niles can't think yet: choose and set up a thinking provider. {} needs an API key.",
                    label(self.llm.provider_in_use())
                ),
            ),
            (
                "stt.api_key",
                Severity::Degraded,
                format!(
                    "Niles can't hear yet: choose and set up a hearing provider. {} needs an API key.",
                    label(self.stt.provider_in_use())
                ),
            ),
        ] {
            if self.secret_source(key) == crate::Source::Unset {
                gaps.push(Gap {
                    path: key,
                    severity,
                    consequence,
                });
            }
        }

        // Said until somebody chooses again: the house still works, on
        // a provider nobody picked.
        for retired in &self.retired {
            match retired {
                Retired::Role { section, provider } => {
                    let (path, job, role) = if *section == "stt" {
                        ("stt.provider", "Hearing", crate::Role::Stt)
                    } else {
                        ("llm.provider", "Thinking", crate::Role::Llm)
                    };
                    gaps.push(Gap {
                        path,
                        severity: Severity::Degraded,
                        consequence: format!(
                            "{job} was set to {provider}, which Niles no longer supports, so it is using {} for now. Choose a provider to keep it.",
                            crate::catalogue::default_provider(role).label
                        ),
                    });
                }
                Retired::Tier2(provider) => gaps.push(Gap {
                    path: "llm.tier2",
                    severity: Severity::Degraded,
                    consequence: format!(
                        "The escalation tier was set to {provider}, which Niles no longer supports, so it is off."
                    ),
                }),
                Retired::Linear => gaps.push(Gap {
                    path: "tool_servers.linear",
                    severity: Severity::Degraded,
                    consequence: "Linear now works through its own tools, and was set up the old way. Add it again under Add-ons, with an API key.".into(),
                }),
                // Nothing to answer: the row could serve nothing, and
                // the history now keeps itself.
                Retired::Provider(_) | Retired::History => {}
            }
        }

        // UTC is the default because guessing a zone would put the
        // curve an hour out and look deliberate. Left as UTC, the curve
        // runs to the wrong clock — which is worth saying out loud.
        if self.home.timezone == "UTC" {
            gaps.push(Gap {
                path: "home.timezone",
                severity: Severity::Degraded,
                consequence: "The curve and the morning routine run on UTC, not your clock.".into(),
            });
        }
        if self.home.latitude == 0.0 && self.home.longitude == 0.0 {
            gaps.push(Gap {
                path: "home.latitude",
                severity: Severity::Degraded,
                consequence: "Weather answers are for the Gulf of Guinea.".into(),
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
        assert!(!cfg.lighting.enabled);
        assert_eq!(cfg.lighting.color_temp_anchors.len(), 8);
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
    fn a_stored_credential_is_not_a_missing_one() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        // The bug this replaced: gaps were decided by whether the
        // config *named* an environment variable, so a broker password
        // typed into Settings worked while the page led with "Niles is
        // missing 2 things it needs".
        crate::secrets::load(std::collections::HashMap::from([
            ("mqtt.username".to_string(), "u".to_string()),
            ("mqtt.password".to_string(), "p".to_string()),
        ]));
        let cfg = Config::load_from_str(
            "[mqtt]
host = \"broker\"
",
        )
        .unwrap();
        let paths: Vec<_> = cfg.setup_gaps().iter().map(|g| g.path).collect();
        assert!(!paths.contains(&"mqtt.username"), "{paths:?}");
        assert!(!paths.contains(&"mqtt.password"), "{paths:?}");
        crate::secrets::load(std::collections::HashMap::new());
    }

    #[test]
    fn a_role_served_by_a_provider_is_not_a_gap_either() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        // One level further along: the key belongs to the provider, and
        // the role has none of its own.
        crate::secrets::load(std::collections::HashMap::from([
            ("provider.elevenlabs.api_key".to_string(), "key".to_string()),
            ("provider.cerebras.api_key".to_string(), "key".to_string()),
        ]));
        let cfg = Config::load_from_str(concat!(
            "[[providers]]
name = \"elevenlabs\"
",
            "base_url = \"https://api.elevenlabs.io/v1\"
serves = [\"stt\"]
",
            "[[providers]]
name = \"cerebras\"
",
            "base_url = \"https://api.cerebras.ai/v1\"
serves = [\"llm\"]
",
            "[stt]
provider = \"elevenlabs\"
[llm]
provider = \"cerebras\"
",
        ))
        .unwrap();
        let paths: Vec<_> = cfg.setup_gaps().iter().map(|g| g.path).collect();
        assert!(!paths.contains(&"stt.api_key"), "{paths:?}");
        assert!(!paths.contains(&"llm.api_key"), "{paths:?}");
        crate::secrets::load(std::collections::HashMap::new());
    }

    #[test]
    fn a_broker_that_is_set_is_not_a_gap() {
        // The variables have to hold something. Naming one that was
        // never set is precisely the half-configured state this
        // reports, so the old version of this test — which named `U`
        // and `P` and set neither — proved nothing.
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var("NILES_TEST_GAP_U", "u");
            std::env::set_var("NILES_TEST_GAP_P", "p");
        }
        let toml = concat!(
            "[mqtt]\nhost = \"192.168.42.16\"\n",
            "username_env = \"NILES_TEST_GAP_U\"\npassword_env = \"NILES_TEST_GAP_P\"\n",
        );
        let gaps = Config::load_from_str(toml).unwrap().setup_gaps();
        assert!(
            !gaps.iter().any(|g| g.path.starts_with("mqtt.")),
            "{gaps:?}"
        );
    }

    #[test]
    fn nothing_left_to_answer_reads_as_set_up() {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var("NILES_TEST_DONE_U", "u");
            std::env::set_var("NILES_TEST_DONE_P", "p");
            std::env::set_var("NILES_TEST_DONE_K", "k");
        }
        let toml = concat!(
            "[home]\nname = \"Home\"\nlatitude = 56.1572\nlongitude = 10.2107\n",
            "timezone = \"Europe/Copenhagen\"\n",
            "[mqtt]\nhost = \"broker\"\n",
            "username_env = \"NILES_TEST_DONE_U\"\npassword_env = \"NILES_TEST_DONE_P\"\n",
            "[stt]\napi_key_env = \"NILES_TEST_DONE_K\"\n",
            "[llm]\napi_key_env = \"NILES_TEST_DONE_K\"\n",
        );
        let cfg = Config::load_from_str(toml).unwrap();
        assert!(cfg.is_set_up(), "{:?}", cfg.setup_gaps());
    }

    #[test]
    fn a_new_install_is_told_which_accounts_it_needs() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        crate::secrets::load(std::collections::HashMap::new());
        let gaps = Config::load_from_str("").unwrap().setup_gaps();
        let said = |path: &str| {
            gaps.iter()
                .find(|g| g.path == path)
                .map(|g| g.consequence.clone())
                .unwrap_or_default()
        };
        assert!(said("mqtt.host").contains("can't reach any lights"));
        assert!(said("stt.api_key").contains("can't hear yet"));
        assert!(said("stt.api_key").contains("ElevenLabs"));
        assert!(said("llm.api_key").contains("can't think yet"));
        assert!(said("llm.api_key").contains("Cerebras"));
    }

    #[test]
    fn a_key_for_the_default_provider_answers_the_gap() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        // Typed into Voice & AI beside the provider, before anybody has
        // declared it: the role is already using it, so it counts.
        crate::secrets::load(std::collections::HashMap::from([
            ("provider.elevenlabs.api_key".to_string(), "key".to_string()),
            ("provider.cerebras.api_key".to_string(), "key".to_string()),
        ]));
        let paths: Vec<_> = Config::load_from_str("")
            .unwrap()
            .setup_gaps()
            .iter()
            .map(|g| g.path)
            .collect();
        assert!(!paths.contains(&"stt.api_key"), "{paths:?}");
        assert!(!paths.contains(&"llm.api_key"), "{paths:?}");
        crate::secrets::load(std::collections::HashMap::new());
    }

    #[test]
    fn a_role_that_fell_back_from_groq_says_so_until_somebody_chooses() {
        let cfg = Config::load_from_str(concat!(
            "[[providers]]
name = \"groq\"
",
            "base_url = \"https://api.groq.com/openai/v1\"
",
            "[stt]
provider = \"groq\"
",
            "[llm.tier2]
provider = \"groq\"
",
        ))
        .unwrap();
        let gaps = cfg.setup_gaps();
        let hearing = gaps
            .iter()
            .find(|g| g.path == "stt.provider")
            .expect("said");
        assert!(
            hearing.consequence.contains("groq"),
            "{}",
            hearing.consequence
        );
        assert!(
            hearing.consequence.contains("ElevenLabs"),
            "{}",
            hearing.consequence
        );
        assert!(gaps.iter().any(|g| g.path == "llm.tier2"), "{gaps:?}");
        assert!(
            !gaps.iter().any(|g| g.path == "llm.provider"),
            "thinking never named it: {gaps:?}"
        );
    }
}

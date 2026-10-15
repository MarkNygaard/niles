//! `[[providers]]` — the outside services Niles has an account with.
//!
//! A provider is a place with an endpoint and a key. A *role* —
//! speech-to-text, the language model, its escalation tier — points at
//! one and brings its own model name.
//!
//! Splitting them that way removes a duplication that was there from
//! the start: one account served both speech and language, and each
//! section carried its own copy of the same URL and the same key. It
//! also makes the escalation tier ordinary rather than special — it was
//! the one role that already had to name a different provider, and did
//! so by repeating the whole shape inline.
//!
//! What stays with the role is the model, because a model belongs to a
//! role and not to an account.
//!
//! Only providers in the [catalogue](crate::catalogue) are kept. A row
//! for one Niles no longer ships — Groq, in configs written before it
//! was removed — is dropped as the config loads, and a role that named
//! it goes back to its default; see [`crate::Config::settle`].

use crate::catalogue;
use crate::error::{Error, Result};
use serde::Deserialize;

/// One account with an inference provider.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// What roles refer to it by — `"cerebras"`. Lowercase, and unique.
    pub name: String,

    /// Where its API lives.
    pub base_url: String,

    /// Which roles this provider can serve.
    ///
    /// Declared rather than assumed, because they genuinely differ: a
    /// provider that only does language inference has no speech
    /// endpoint at all. Offering it for speech would produce a 404 from
    /// somebody else's server, which is a poor way to find out.
    ///
    /// Empty means "anything", which is the honest default for a
    /// provider nobody has described.
    #[serde(default)]
    pub serves: Vec<Role>,
}

/// What a provider can be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Role {
    Stt,
    Llm,
}

impl ProviderConfig {
    /// Whether this provider can serve `role`. An undescribed provider
    /// is assumed able to, rather than assumed unable — refusing to use
    /// something because nobody wrote down what it does is worse than
    /// letting it fail with the provider's own error.
    pub fn serves(&self, role: Role) -> bool {
        self.serves.is_empty() || self.serves.contains(&role)
    }

    /// Where this provider's key is kept.
    pub fn secret_key(&self) -> String {
        format!("provider.{}.api_key", self.name)
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(invalid("a provider must have a name"));
        }
        if self.name != self.name.to_lowercase() {
            return Err(invalid(format!(
                "provider name {:?} must be lowercase, so roles refer to it the same way twice",
                self.name
            )));
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(invalid(format!(
                "provider {:?} has a base_url that is not a URL: {:?}",
                self.name, self.base_url
            )));
        }
        Ok(())
    }
}

/// Find a provider by name.
pub fn find<'a>(providers: &'a [ProviderConfig], name: &str) -> Option<&'a ProviderConfig> {
    providers.iter().find(|p| p.name == name)
}

/// Check the whole list: every one valid, and no two sharing a name.
pub fn validate_all(providers: &[ProviderConfig]) -> Result<()> {
    for provider in providers {
        provider.validate()?;
    }
    let mut seen: Vec<&str> = Vec::with_capacity(providers.len());
    for provider in providers {
        if seen.contains(&provider.name.as_str()) {
            return Err(invalid(format!(
                "two providers are both called {:?}, so a role naming it would be ambiguous",
                provider.name
            )));
        }
        seen.push(&provider.name);
    }
    Ok(())
}

/// Where a role's requests go, and with what key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
}

/// Something a loaded config named that Niles no longer has, and what
/// it does instead.
///
/// Kept on the config rather than only logged, so the setup page can
/// say it until somebody chooses again.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retired {
    /// A `[[providers]]` row for a provider not in the catalogue. It is
    /// dropped: nothing could use it.
    Provider(String),
    /// A role named a provider that is not declared. It uses its
    /// default provider and model instead.
    Role {
        section: &'static str,
        provider: String,
    },
    /// The escalation tier named a provider that is not declared. It is
    /// switched off rather than sent to OpenAI with a key nobody gave.
    Tier2(String),
    /// `[integrations.linear]`, from before Linear's tools came from
    /// Linear's own MCP server. Dropped: Linear is added again as a
    /// tool server, with its own key.
    Linear,
}

impl crate::Config {
    /// Clear out what a stored config names that Niles no longer has.
    ///
    /// A one-off for real deployments that set Groq up before it was
    /// removed: their stored overrides still declare it, and may still
    /// point a role at it. Refusing to load would take the house down
    /// over a setting nobody can now change, so each is dropped or set
    /// back to its default, and recorded in [`Self::retired`].
    pub(crate) fn settle(&mut self) {
        let mut retired = Vec::new();
        self.providers.retain(|p| {
            let known =
                catalogue::find(&p.name).is_some_and(|k| k.kind == catalogue::Kind::Provider);
            if !known {
                retired.push(Retired::Provider(p.name.clone()));
            }
            known
        });
        let declared: Vec<String> = self.providers.iter().map(|p| p.name.clone()).collect();
        let missing = |named: &Option<String>| {
            named
                .as_ref()
                .filter(|name| !declared.contains(name))
                .cloned()
        };

        if let Some(provider) = missing(&self.stt.provider) {
            self.stt.provider = None;
            self.stt.base_url = None;
            // The model was chosen for the provider that is gone.
            self.stt.model = catalogue::default_model(Role::Stt).into();
            retired.push(Retired::Role {
                section: "stt",
                provider,
            });
        }
        if let Some(provider) = missing(&self.llm.provider) {
            self.llm.provider = None;
            self.llm.base_url = None;
            self.llm.model = catalogue::default_model(Role::Llm).into();
            retired.push(Retired::Role {
                section: "llm",
                provider,
            });
        }
        if let Some(provider) = self
            .llm
            .tier2
            .as_ref()
            .and_then(|tier2| missing(&tier2.provider))
        {
            self.llm.tier2 = None;
            retired.push(Retired::Tier2(provider));
        }
        if self.integrations.linear.take().is_some() {
            retired.push(Retired::Linear);
        }
        self.retired = retired;
    }

    /// Where a role's requests go, without its key.
    ///
    /// A named provider wins outright: pointing a role at one is a
    /// deliberate act, and quietly preferring the role's own leftover
    /// URL would make the setting look ignored. Naming none uses the
    /// role's own endpoint when it has one, and its default provider's
    /// when it does not.
    pub fn base_url_for(
        &self,
        role: Role,
        provider: Option<&str>,
        own_base_url: Option<&str>,
    ) -> Option<String> {
        match (provider, own_base_url) {
            (Some(name), _) => find(&self.providers, name).map(|p| p.base_url.clone()),
            (None, Some(url)) => Some(url.to_string()),
            (None, None) => catalogue::default_provider(role).base_url.map(Into::into),
        }
    }

    /// Resolve a role to an endpoint.
    ///
    /// A provider's key is the provider's, which is the whole point —
    /// one account, one key, however many roles use it. A role on its
    /// own endpoint uses its own key.
    pub fn endpoint_for(
        &self,
        role: Role,
        provider: Option<&str>,
        own_base_url: Option<&str>,
        own_key_env: &str,
        section: &'static str,
    ) -> Result<Endpoint> {
        let own_key = format!("{section}.api_key");
        match (provider, own_base_url) {
            (None, Some(url)) => Ok(Endpoint {
                base_url: url.to_string(),
                api_key: crate::env::require_secret(section, &own_key, own_key_env)?,
            }),
            (None, None) => {
                let known = catalogue::default_provider(role);
                let key = known.secret_key().expect("a provider keeps a key");
                Ok(Endpoint {
                    base_url: known
                        .base_url
                        .expect("a default provider has an endpoint")
                        .to_string(),
                    // The role's own stored key is honoured as well:
                    // somebody who typed one in before the role had a
                    // default provider should not have to type it again.
                    api_key: crate::env::require_secret(section, &key, own_key_env)
                        .or_else(|_| crate::env::require_secret(section, &own_key, own_key_env))?,
                })
            }
            (Some(name), _) => {
                let found = find(&self.providers, name).ok_or_else(|| {
                    invalid(format!(
                        "{section} names the provider {name:?}, and no [[providers]] entry is called that"
                    ))
                })?;
                if !found.serves(role) {
                    return Err(invalid(format!(
                        "{section} names the provider {name:?}, which does not serve it"
                    )));
                }
                Ok(Endpoint {
                    base_url: found.base_url.clone(),
                    // The role's own env var is still honoured as a last
                    // resort, so a provider can be adopted before its key
                    // has been moved across.
                    api_key: crate::env::require_secret(section, &found.secret_key(), own_key_env)?,
                })
            }
        }
    }
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::InvalidSection {
        section: "providers",
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cerebras() -> ProviderConfig {
        ProviderConfig {
            name: "cerebras".into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            serves: vec![Role::Llm],
        }
    }

    /// A stored config as a deployment that set Groq up has it: the row,
    /// and both roles pointed at it.
    const WITH_GROQ: &str = concat!(
        "[[providers]]\nname = \"groq\"\n",
        "base_url = \"https://api.groq.com/openai/v1\"\nserves = [\"stt\", \"llm\"]\n",
        "[[providers]]\nname = \"cerebras\"\n",
        "base_url = \"https://api.cerebras.ai/v1\"\nserves = [\"llm\"]\n",
        "[stt]\nprovider = \"groq\"\nmodel = \"whisper-large-v3-turbo\"\n",
        "[llm]\nprovider = \"groq\"\nmodel = \"openai/gpt-oss-20b\"\nreasoning_effort = \"low\"\n",
    );

    #[test]
    fn a_provider_keeps_its_key_under_its_own_name() {
        assert_eq!(cerebras().secret_key(), "provider.cerebras.api_key");
    }

    #[test]
    fn a_provider_serves_only_what_it_says_it_does() {
        let llm_only = cerebras();
        assert!(llm_only.serves(Role::Llm));
        assert!(
            !llm_only.serves(Role::Stt),
            "offering it for speech would 404 on somebody else's server"
        );
    }

    #[test]
    fn an_undescribed_provider_is_assumed_able() {
        // Refusing to use something because nobody wrote down what it
        // does is worse than letting it fail with its own error.
        let bare = ProviderConfig {
            serves: vec![],
            ..cerebras()
        };
        assert!(bare.serves(Role::Stt) && bare.serves(Role::Llm));
    }

    #[test]
    fn two_providers_cannot_share_a_name() {
        let err = validate_all(&[cerebras(), cerebras()]).unwrap_err();
        assert!(format!("{err}").contains("ambiguous"), "{err}");
    }

    #[test]
    fn a_name_that_is_not_lowercase_is_refused() {
        // Otherwise `provider = "Cerebras"` and `provider = "cerebras"`
        // are two different providers with one key between them.
        let mut shouty = cerebras();
        shouty.name = "Cerebras".into();
        assert!(shouty.validate().is_err());
    }

    #[test]
    fn a_base_url_that_is_not_a_url_is_refused() {
        let mut wrong = cerebras();
        wrong.base_url = "api.cerebras.ai".into();
        assert!(wrong.validate().is_err());
    }

    #[test]
    fn a_role_that_names_nothing_uses_its_default_provider() {
        let _guard = crate::secrets::TEST_GUARD.lock();
        crate::secrets::load(std::collections::HashMap::from([
            ("provider.elevenlabs.api_key".into(), "el-key".into()),
            ("provider.cerebras.api_key".into(), "cb-key".into()),
        ]));
        let cfg = crate::Config::load_from_str("").expect("valid");
        let hearing = cfg
            .endpoint_for(Role::Stt, None, None, "", "stt")
            .expect("resolves");
        assert_eq!(hearing.base_url, "https://api.elevenlabs.io/v1");
        assert_eq!(hearing.api_key, "el-key");
        let thinking = cfg
            .endpoint_for(Role::Llm, None, None, "", "llm")
            .expect("resolves");
        assert_eq!(thinking.base_url, "https://api.cerebras.ai/v1");
        assert_eq!(thinking.api_key, "cb-key");
        crate::secrets::load(std::collections::HashMap::new());
    }

    #[test]
    fn a_role_with_its_own_endpoint_uses_its_own_key() {
        // A Whisper server in the house: no provider, its own URL.
        unsafe { std::env::set_var("NILES_TEST_PROV_OWN", "own-key") };
        let cfg = crate::Config::load_from_str(
            "[stt]\nbase_url = \"http://whisper.local/v1\"\napi_key_env = \"NILES_TEST_PROV_OWN\"\n",
        )
        .expect("valid");
        let endpoint = cfg
            .endpoint_for(
                Role::Stt,
                cfg.stt.provider.as_deref(),
                cfg.stt.base_url.as_deref(),
                &cfg.stt.api_key_env,
                "stt",
            )
            .expect("resolves");
        assert_eq!(endpoint.base_url, "http://whisper.local/v1");
        assert_eq!(endpoint.api_key, "own-key");
        assert_eq!(cfg.stt.provider_in_use(), None);
    }

    #[test]
    fn naming_a_provider_takes_its_endpoint() {
        unsafe { std::env::set_var("NILES_TEST_PROV_KEY", "shared-key") };
        let cfg = crate::Config::load_from_str(concat!(
            "[[providers]]\nname = \"cerebras\"\n",
            "base_url = \"https://api.cerebras.ai/v1\"\n",
            "[llm]\nprovider = \"cerebras\"\nbase_url = \"https://leftover.test/v1\"\n",
            "api_key_env = \"NILES_TEST_PROV_KEY\"\n",
        ))
        .expect("valid");
        let endpoint = cfg
            .endpoint_for(
                Role::Llm,
                cfg.llm.provider.as_deref(),
                cfg.llm.base_url.as_deref(),
                &cfg.llm.api_key_env,
                "llm",
            )
            .expect("resolves");
        assert_eq!(
            endpoint.base_url, "https://api.cerebras.ai/v1",
            "the provider's endpoint, not the section's leftover"
        );
    }

    #[test]
    fn naming_a_provider_that_does_not_exist_says_so() {
        let cfg = crate::Config::load_from_str("").expect("valid");
        let err = cfg
            .endpoint_for(Role::Llm, Some("nobody"), None, "", "llm")
            .unwrap_err();
        assert!(format!("{err}").contains("no [[providers]] entry"), "{err}");
    }

    #[test]
    fn a_provider_cannot_be_used_for_a_role_it_does_not_serve() {
        // The whole reason `serves` is declared: this fails here with a
        // sentence, rather than as a 404 from somebody else's server.
        let cfg = crate::Config::load_from_str(concat!(
            "[[providers]]\nname = \"cerebras\"\n",
            "base_url = \"https://api.cerebras.ai/v1\"\nserves = [\"llm\"]\n",
        ))
        .expect("valid");
        let err = cfg
            .endpoint_for(Role::Stt, Some("cerebras"), None, "", "stt")
            .unwrap_err();
        assert!(format!("{err}").contains("does not serve it"), "{err}");
    }

    #[test]
    fn a_stored_groq_row_is_dropped_and_said() {
        let cfg = crate::Config::load_from_str(WITH_GROQ).expect("loads");
        cfg.validate().expect("and is valid");
        let names: Vec<&str> = cfg.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["cerebras"], "the rest are kept");
        assert!(cfg.retired.contains(&Retired::Provider("groq".into())));
    }

    #[test]
    fn a_role_that_pointed_at_groq_falls_back_to_its_default() {
        let cfg = crate::Config::load_from_str(WITH_GROQ).expect("loads");
        assert_eq!(cfg.stt.provider, None);
        assert_eq!(cfg.stt.provider_in_use(), Some("elevenlabs"));
        assert_eq!(
            cfg.stt.model, "scribe_v2",
            "Groq's model is not ElevenLabs's"
        );
        assert_eq!(cfg.llm.provider, None);
        assert_eq!(cfg.llm.provider_in_use(), Some("cerebras"));
        assert_eq!(cfg.llm.model, "gpt-oss-120b");
        assert!(cfg.retired.contains(&Retired::Role {
            section: "stt",
            provider: "groq".into()
        }));
        assert!(cfg.retired.contains(&Retired::Role {
            section: "llm",
            provider: "groq".into()
        }));
    }

    #[test]
    fn a_role_on_a_provider_that_is_still_there_is_left_alone() {
        let cfg = crate::Config::load_from_str(concat!(
            "[[providers]]\nname = \"cerebras\"\n",
            "base_url = \"https://api.cerebras.ai/v1\"\nserves = [\"llm\"]\n",
            "[llm]\nprovider = \"cerebras\"\nmodel = \"qwen-3.8-27b\"\n",
        ))
        .expect("loads");
        assert_eq!(cfg.llm.provider.as_deref(), Some("cerebras"));
        assert_eq!(cfg.llm.model, "qwen-3.8-27b");
        assert!(cfg.retired.is_empty(), "{:?}", cfg.retired);
    }

    #[test]
    fn an_escalation_tier_on_groq_is_switched_off() {
        // Not sent to OpenAI instead: that would need a key nobody gave,
        // and a tier that cannot be built stops Niles starting.
        let cfg = crate::Config::load_from_str(
            "[llm.tier2]\nprovider = \"groq\"\nmodel = \"llama-3.3-70b-versatile\"\n",
        )
        .expect("loads");
        assert!(cfg.llm.tier2.is_none());
        assert!(cfg.retired.contains(&Retired::Tier2("groq".into())));
    }

    #[test]
    fn the_store_loads_a_stored_document_with_groq_in_it() {
        // The path a real deployment takes: the overrides come from
        // Postgres, layered over the file.
        let store = crate::ConfigStore::from_str_in_memory("").expect("base");
        let overrides: toml::Table = toml::from_str(WITH_GROQ).expect("toml");
        store
            .adopt(crate::StoredState {
                overrides,
                revisions: Vec::new(),
            })
            .expect("a stored Groq setup does not stop Niles loading");
        let cfg = store.current();
        assert!(cfg.providers.iter().all(|p| p.name != "groq"));
        assert_eq!(cfg.stt.provider_in_use(), Some("elevenlabs"));
        assert_eq!(cfg.llm.provider_in_use(), Some("cerebras"));
        assert!(!cfg.retired.is_empty());
    }
}

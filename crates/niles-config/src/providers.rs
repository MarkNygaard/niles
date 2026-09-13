//! `[[providers]]` — the outside services Niles has an account with.
//!
//! A provider is a place with an endpoint and a key. A *role* —
//! speech-to-text, the language model, its escalation tier — points at
//! one and brings its own model name.
//!
//! Splitting them that way removes a duplication that was there from
//! the start: one Groq account served both speech and language, and
//! each section carried its own copy of the same URL and the same key.
//! It also makes the escalation tier ordinary rather than special — it
//! was the one role that already had to name a different provider, and
//! did so by repeating the whole shape inline.
//!
//! What stays with the role is the model, because a model belongs to a
//! role and not to an account: the same Groq key serves
//! `whisper-large-v3-turbo` and `openai/gpt-oss-20b`.

use crate::error::{Error, Result};
use serde::Deserialize;

/// One account with an inference provider.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// What roles refer to it by — `"groq"`. Lowercase, and unique.
    pub name: String,

    /// Where its API lives. OpenAI-compatible, which is what lets one
    /// client talk to all of them.
    pub base_url: String,

    /// Which roles this provider can serve.
    ///
    /// Declared rather than assumed, because they genuinely differ:
    /// Groq does speech and language, and a provider that only does
    /// language inference has no speech endpoint at all. Offering it
    /// for speech would produce a 404 from somebody else's server,
    /// which is a poor way to find out.
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

impl crate::Config {
    /// Resolve a role to an endpoint.
    ///
    /// A named provider wins outright: pointing a role at one is a
    /// deliberate act, and quietly preferring the role's own leftover
    /// URL would make the setting look ignored. Its key is the
    /// provider's, which is the whole point — one Groq account, one
    /// key, however many roles use it.
    ///
    /// Naming no provider falls back to the role's own `base_url` and
    /// key, which is what every config written before this existed
    /// does, and what keeps them working untouched.
    pub fn endpoint_for(
        &self,
        role: Role,
        provider: Option<&str>,
        own_base_url: &str,
        own_key_env: &str,
        section: &'static str,
    ) -> Result<Endpoint> {
        let Some(name) = provider else {
            return Ok(Endpoint {
                base_url: own_base_url.to_string(),
                api_key: crate::env::require_secret(
                    section,
                    &format!("{section}.api_key"),
                    own_key_env,
                )?,
            });
        };

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
            // resort, so a provider can be adopted before its key has
            // been moved across.
            api_key: crate::env::require_secret(section, &found.secret_key(), own_key_env)?,
        })
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

    fn groq() -> ProviderConfig {
        ProviderConfig {
            name: "groq".into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            serves: vec![Role::Stt, Role::Llm],
        }
    }

    #[test]
    fn a_provider_keeps_its_key_under_its_own_name() {
        assert_eq!(groq().secret_key(), "provider.groq.api_key");
    }

    #[test]
    fn a_provider_serves_only_what_it_says_it_does() {
        let llm_only = ProviderConfig {
            name: "cerebras".into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            serves: vec![Role::Llm],
        };
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
            name: "somewhere".into(),
            base_url: "https://example.test/v1".into(),
            serves: vec![],
        };
        assert!(bare.serves(Role::Stt) && bare.serves(Role::Llm));
    }

    #[test]
    fn two_providers_cannot_share_a_name() {
        let err = validate_all(&[groq(), groq()]).unwrap_err();
        assert!(format!("{err}").contains("ambiguous"), "{err}");
    }

    #[test]
    fn a_name_that_is_not_lowercase_is_refused() {
        // Otherwise `provider = "Groq"` and `provider = "groq"` are two
        // different providers with one key between them.
        let mut shouty = groq();
        shouty.name = "Groq".into();
        assert!(shouty.validate().is_err());
    }

    #[test]
    fn a_role_that_names_no_provider_behaves_as_before() {
        // The migration property. Every config written before this
        // existed names no provider, and has to keep working untouched.
        unsafe { std::env::set_var("NILES_TEST_PROV_OLD", "old-key") };
        let cfg = crate::Config::load_from_str(
            "[stt]
api_key_env = \"NILES_TEST_PROV_OLD\"
",
        )
        .expect("valid");
        let endpoint = cfg
            .endpoint_for(
                Role::Stt,
                None,
                &cfg.stt.base_url,
                &cfg.stt.api_key_env,
                "stt",
            )
            .expect("resolves");
        assert_eq!(endpoint.base_url, "https://api.groq.com/openai/v1");
        assert_eq!(endpoint.api_key, "old-key");
    }

    #[test]
    fn naming_a_provider_takes_its_endpoint() {
        unsafe { std::env::set_var("NILES_TEST_PROV_KEY", "shared-key") };
        let cfg = crate::Config::load_from_str(concat!(
            "[[providers]]
name = \"somewhere\"
",
            "base_url = \"https://example.test/v1\"
",
            "[stt]
provider = \"somewhere\"
api_key_env = \"NILES_TEST_PROV_KEY\"
",
        ))
        .expect("valid");
        let endpoint = cfg
            .endpoint_for(
                Role::Stt,
                cfg.stt.provider.as_deref(),
                &cfg.stt.base_url,
                &cfg.stt.api_key_env,
                "stt",
            )
            .expect("resolves");
        assert_eq!(
            endpoint.base_url, "https://example.test/v1",
            "the provider's endpoint, not the section's leftover default"
        );
    }

    #[test]
    fn naming_a_provider_that_does_not_exist_says_so() {
        let cfg = crate::Config::load_from_str(
            "[llm]
provider = \"nobody\"
",
        )
        .expect("valid");
        let err = cfg
            .endpoint_for(Role::Llm, Some("nobody"), "", "", "llm")
            .unwrap_err();
        assert!(format!("{err}").contains("no [[providers]] entry"), "{err}");
    }

    #[test]
    fn a_provider_cannot_be_used_for_a_role_it_does_not_serve() {
        // The whole reason `serves` is declared: this fails here with a
        // sentence, rather than as a 404 from somebody else's server.
        let cfg = crate::Config::load_from_str(concat!(
            "[[providers]]
name = \"cerebras\"
",
            "base_url = \"https://api.cerebras.ai/v1\"
serves = [\"llm\"]
",
        ))
        .expect("valid");
        let err = cfg
            .endpoint_for(Role::Stt, Some("cerebras"), "", "", "stt")
            .unwrap_err();
        assert!(format!("{err}").contains("does not serve it"), "{err}");
    }

    #[test]
    fn a_base_url_that_is_not_a_url_is_refused() {
        let mut wrong = groq();
        wrong.base_url = "api.groq.com".into();
        assert!(wrong.validate().is_err());
    }
}

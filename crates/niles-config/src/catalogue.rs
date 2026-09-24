//! What Niles has been taught to talk to.
//!
//! Not a config section — a list of facts about the outside world that
//! belong to the code rather than to anybody's file. Whether Niles can
//! use Groq is decided by the client it ships with; where Groq's API
//! lives has exactly one answer; and which models it serves is
//! something a person should pick from rather than be asked to
//! remember.
//!
//! Keeping the list here rather than in the API crate means the shipped
//! defaults can read from it. `[stt]` with nothing in it has to name a
//! model, and that model and the one offered in the dropdown must be
//! the same string — which they now are by construction rather than by
//! somebody noticing.

use crate::providers::Role;

/// How a thing is set up, which decides what its card looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An inference account: one endpoint, one key, and the roles it
    /// can serve. Adding it appends to `[[providers]]`.
    Provider,
    /// Anything with its own wiring — an OAuth flow, a webhook.
    Service,
}

/// Which API a provider speaks.
///
/// Every provider here but one answers the OpenAI shape at its base URL,
/// which is why a provider has always been a URL and a key. ElevenLabs
/// does not, so the catalogue has to say which client to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    OpenAi,
    ElevenLabs,
}

pub struct Known {
    pub id: &'static str,
    pub label: &'static str,
    pub blurb: &'static str,
    pub kind: Kind,
    /// Providers only. The one address its API lives at, so nobody has
    /// to look it up or mistype it.
    pub base_url: Option<&'static str>,
    pub serves: &'static [Role],
    /// Services have none and say `OpenAi`, which nothing reads.
    pub api: Api,
    /// What it can be asked for, per role. **The first entry for a role
    /// is what Niles uses when nothing is written down.**
    ///
    /// A list rather than a free-text box because nobody remembers
    /// `distil-whisper-large-v3-en`, and a box you have to guess at is
    /// a box that gets a wrong answer typed into it. It will go stale —
    /// providers add models — so the page keeps whatever is already
    /// configured even when it is not on this list, and offers a way to
    /// type one that is not.
    pub models: &'static [(Role, &'static str)],
}

/// The provider the shipped defaults point at.
///
/// `[stt]` and `[llm]` have to default to *something*, and this is what
/// they have always defaulted to.
pub const DEFAULT_PROVIDER: &str = "groq";

pub const KNOWN: &[Known] = &[
    Known {
        id: "groq",
        label: "Groq",
        blurb: "Speech-to-text and language models, fast enough for a house.",
        kind: Kind::Provider,
        base_url: Some("https://api.groq.com/openai/v1"),
        serves: &[Role::Stt, Role::Llm],
        api: Api::OpenAi,
        models: &[
            (Role::Stt, "whisper-large-v3-turbo"),
            (Role::Stt, "whisper-large-v3"),
            (Role::Stt, "distil-whisper-large-v3-en"),
            (Role::Llm, "openai/gpt-oss-20b"),
            (Role::Llm, "openai/gpt-oss-120b"),
            (Role::Llm, "llama-3.3-70b-versatile"),
            (Role::Llm, "llama-3.1-8b-instant"),
        ],
    },
    Known {
        id: "cerebras",
        label: "Cerebras",
        blurb: "Language models on wafer-scale silicon — the fastest way to answer out loud.",
        kind: Kind::Provider,
        base_url: Some("https://api.cerebras.ai/v1"),
        // Language only. There is no speech endpoint here, and
        // offering one would fail as a 404 from somebody else's
        // server rather than as something this page could explain.
        serves: &[Role::Llm],
        api: Api::OpenAi,
        models: &[(Role::Llm, "gpt-oss-120b"), (Role::Llm, "qwen-3.8-27b")],
    },
    Known {
        id: "elevenlabs",
        label: "ElevenLabs",
        blurb: "Scribe speech-to-text: listens for Niles's own name, and scores every word.",
        kind: Kind::Provider,
        base_url: Some("https://api.elevenlabs.io/v1"),
        serves: &[Role::Stt],
        api: Api::ElevenLabs,
        models: &[(Role::Stt, "scribe_v2")],
    },
    Known {
        id: "tado",
        label: "tado°",
        blurb: "Who is home, from the thermostats that already know.",
        kind: Kind::Service,
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "unifi",
        label: "UniFi",
        blurb: "Who is home, from the Wi-Fi — the moment a phone is in range.",
        kind: Kind::Service,
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "linear",
        label: "Linear",
        blurb: "Turns an issue into work Niles picks up.",
        kind: Kind::Service,
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
];

pub fn find(id: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|known| known.id == id)
}

impl Known {
    /// Everything it offers for a role, in the order it is offered.
    pub fn models_for(&self, role: Role) -> impl Iterator<Item = &'static str> {
        self.models
            .iter()
            .filter(move |(r, _)| *r == role)
            .map(|(_, model)| *model)
    }

    /// Where its key is kept.
    pub fn secret_key(&self) -> Option<String> {
        match self.kind {
            Kind::Provider => Some(format!("provider.{}.api_key", self.id)),
            // tado has tokens rather than a key, and fetches them itself.
            Kind::Service if self.id == "linear" => Some("integrations.linear.api_key".into()),
            Kind::Service if self.id == "unifi" => Some("presence.unifi.api_key".into()),
            Kind::Service => None,
        }
    }
}

/// The model the shipped config uses for a role.
///
/// Panics only if the catalogue is inconsistent with itself — the
/// default provider missing, or serving a role it lists no model for —
/// which the tests below rule out at build time rather than leaving to
/// a first start.
pub fn default_model(role: Role) -> &'static str {
    find(DEFAULT_PROVIDER)
        .and_then(|known| known.models_for(role).next())
        .expect("the default provider serves every role the shipped config has")
}

/// The endpoint the shipped config points a role at.
pub fn default_base_url() -> &'static str {
    find(DEFAULT_PROVIDER)
        .and_then(|known| known.base_url)
        .expect("the default provider is a provider, so it has an endpoint")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevenlabs_is_the_one_that_does_not_speak_openai() {
        // The client Niles builds for speech-to-text is chosen from this;
        // a provider marked wrongly gets requests it cannot parse.
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Provider) {
            let expected = if known.id == "elevenlabs" {
                Api::ElevenLabs
            } else {
                Api::OpenAi
            };
            assert_eq!(known.api, expected, "{}", known.id);
        }
        assert_eq!(
            find("elevenlabs").unwrap().models_for(Role::Stt).next(),
            Some("scribe_v2")
        );
    }

    #[test]
    fn the_default_provider_exists_and_is_one() {
        let known = find(DEFAULT_PROVIDER).expect("in the catalogue");
        assert_eq!(known.kind, Kind::Provider);
        assert!(known.base_url.is_some());
    }

    #[test]
    fn everything_a_provider_serves_it_offers_a_model_for() {
        // A provider that serves a role with no model would leave the
        // dropdown empty and the shipped default unresolvable.
        for known in KNOWN {
            for role in known.serves {
                assert!(
                    known.models_for(*role).next().is_some(),
                    "{} serves {role:?} and offers no model for it",
                    known.id
                );
            }
        }
    }

    #[test]
    fn a_service_offers_no_models_and_no_endpoint() {
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Service) {
            assert!(known.base_url.is_none(), "{}", known.id);
            assert!(known.models.is_empty(), "{}", known.id);
            assert!(known.serves.is_empty(), "{}", known.id);
        }
    }

    #[test]
    fn the_first_model_for_a_role_is_the_default() {
        assert_eq!(default_model(Role::Stt), "whisper-large-v3-turbo");
        assert_eq!(default_model(Role::Llm), "openai/gpt-oss-20b");
    }

    #[test]
    fn a_provider_is_only_offered_for_what_it_serves() {
        // What keeps Cerebras out of the speech dropdown. It has no
        // transcription endpoint, and the settings page filters on
        // exactly this: offering it there would fail as a 404 from
        // somebody else's server.
        let cerebras = find("cerebras").expect("in the catalogue");
        assert_eq!(cerebras.serves, &[Role::Llm]);
        assert!(cerebras.models_for(Role::Stt).next().is_none());
        assert_eq!(cerebras.models_for(Role::Llm).next(), Some("gpt-oss-120b"));
    }

    #[test]
    fn every_provider_has_somewhere_to_keep_a_key() {
        // The Integrations card shows a key field when there is a place
        // to put it, and the route that saves one accepts only keys
        // Niles reads. A provider without this is one the page offers
        // and nobody can finish setting up.
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Provider) {
            assert_eq!(
                known.secret_key().as_deref(),
                Some(format!("provider.{}.api_key", known.id).as_str()),
                "{}",
                known.id
            );
        }
    }

    #[test]
    fn no_two_entries_share_an_id() {
        let mut seen: Vec<&str> = Vec::new();
        for known in KNOWN {
            assert!(!seen.contains(&known.id), "two entries called {}", known.id);
            seen.push(known.id);
        }
    }
}

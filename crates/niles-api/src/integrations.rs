//! What Niles can actually be connected to.
//!
//! The page this feeds used to be a pair of text boxes: a name and a
//! URL. That let somebody type "chatgpt" and a chatgpt.com address and
//! get a provider that would never work, because whether Niles can talk
//! to a service is a question about the code, not about the config —
//! and it also asked for a base URL that Niles already knows, since
//! there is only one address Groq's API lives at.
//!
//! So the list is here instead. Adding one is picking from it, and what
//! the page shows afterwards is only what is set up. A hundred entries
//! later that is still one card per thing you actually use.
//!
//! Adding a new OpenAI-compatible provider is one entry in [`KNOWN`].
//! Anything that needs more than a base URL and a key — tado's device
//! flow, Linear's webhook — is a `Service` and brings its own card.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use niles_config::{Config, Role};

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

struct Known {
    id: &'static str,
    label: &'static str,
    blurb: &'static str,
    kind: Kind,
    /// Providers only. The one address its API lives at, so nobody has
    /// to look it up or mistype it.
    base_url: Option<&'static str>,
    serves: &'static [Role],
}

const KNOWN: &[Known] = &[
    Known {
        id: "groq",
        label: "Groq",
        blurb: "Speech-to-text and language models, fast enough for a house.",
        kind: Kind::Provider,
        base_url: Some("https://api.groq.com/openai/v1"),
        serves: &[Role::Stt, Role::Llm],
    },
    Known {
        id: "tado",
        label: "tado°",
        blurb: "Who is home, from the thermostats that already know.",
        kind: Kind::Service,
        base_url: None,
        serves: &[],
    },
    Known {
        id: "linear",
        label: "Linear",
        blurb: "Turns an issue into work Niles picks up.",
        kind: Kind::Service,
        base_url: None,
        serves: &[],
    },
];

/// One entry as the page sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrationDto {
    pub id: String,
    pub label: String,
    pub blurb: String,
    pub kind: Kind,
    pub base_url: Option<String>,
    pub serves: Vec<Role>,
    /// Whether it is set up. The page shows the ones that are and
    /// offers the rest behind the add button.
    pub added: bool,
    /// Where its key lives, for the card to show a field for. `None`
    /// for anything that does not take one.
    pub secret_key: Option<String>,
}

/// `GET /integrations` — everything Niles knows how to connect to, and
/// which of them are set up.
pub async fn list(State(state): State<AppState>) -> Json<Vec<IntegrationDto>> {
    let cfg = state.config.as_ref().map(|c| c.current());
    Json(
        KNOWN
            .iter()
            .map(|known| IntegrationDto {
                id: known.id.to_string(),
                label: known.label.to_string(),
                blurb: known.blurb.to_string(),
                kind: known.kind,
                base_url: known.base_url.map(Into::into),
                serves: known.serves.to_vec(),
                added: cfg.as_ref().is_some_and(|cfg| is_added(cfg, known.id)),
                secret_key: secret_key(known),
            })
            .collect(),
    )
}

/// Where a given integration keeps its key.
fn secret_key(known: &Known) -> Option<String> {
    match known.kind {
        Kind::Provider => Some(format!("provider.{}.api_key", known.id)),
        // tado has tokens rather than a key, and gets them itself.
        Kind::Service if known.id == "linear" => Some("integrations.linear.api_key".into()),
        Kind::Service => None,
    }
}

/// Whether this config has the integration set up.
///
/// Asked of the config rather than of the secret store: a key on its own
/// connects nothing, and a provider with no key is still a provider
/// somebody added and has yet to finish.
fn is_added(cfg: &Config, id: &str) -> bool {
    match id {
        "tado" => cfg.presence.tado.is_some(),
        "linear" => cfg.integrations.linear.is_some(),
        other => cfg.providers.iter().any(|p| p.name == other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(toml: &str) -> Config {
        Config::load_from_str(toml).expect("valid")
    }

    #[test]
    fn every_entry_says_where_its_key_goes_or_that_it_has_none() {
        for known in KNOWN {
            match known.kind {
                Kind::Provider => {
                    assert!(known.base_url.is_some(), "{} has no endpoint", known.id);
                    assert!(
                        !known.serves.is_empty(),
                        "{} serves nothing, so no role could use it",
                        known.id
                    );
                    assert_eq!(
                        secret_key(known).as_deref(),
                        Some(format!("provider.{}.api_key", known.id).as_str())
                    );
                }
                // A service brings its own wiring, and tado's is tokens
                // it fetches rather than a key anybody types.
                Kind::Service => assert!(known.base_url.is_none(), "{}", known.id),
            }
        }
    }

    #[test]
    fn a_provider_counts_as_added_once_the_config_names_it() {
        let toml = concat!(
            "[[providers]]\n",
            "name = \"groq\"\n",
            "base_url = \"https://api.groq.com/openai/v1\"\n",
        );
        assert!(is_added(&cfg(toml), "groq"));
        assert!(!is_added(&cfg(""), "groq"));
    }

    #[test]
    fn a_provider_with_no_key_yet_is_still_added() {
        // Otherwise adding one and being asked for its key would make
        // the card vanish the moment you looked away from it.
        let toml = concat!(
            "[[providers]]\n",
            "name = \"groq\"\n",
            "base_url = \"https://api.groq.com/openai/v1\"\n",
        );
        assert!(is_added(&cfg(toml), "groq"));
    }

    #[test]
    fn tado_and_linear_are_read_from_their_own_sections() {
        assert!(is_added(
            &cfg("[presence]\nenabled = true\n[presence.tado]\n"),
            "tado"
        ));
        assert!(!is_added(&cfg(""), "tado"));
        assert!(is_added(
            &cfg("[integrations.linear]\nteam = \"niles\"\n"),
            "linear"
        ));
        assert!(!is_added(&cfg(""), "linear"));
    }
}

//! What Niles can actually be connected to.
//!
//! The page this feeds used to be a pair of text boxes: a name and a
//! URL. That let somebody type "chatgpt" and a chatgpt.com address and
//! get a provider that would never work, because whether Niles can talk
//! to a service is a question about the code, not about the config —
//! and it also asked for a base URL that Niles already knows, since
//! there is only one address Groq's API lives at.
//!
//! The list itself lives in `niles-config`, beside the defaults that
//! read from it. This is the route that serves it, plus the one thing
//! the list cannot know on its own: which entries are set up.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use niles_config::catalogue::{self, Known};
use niles_config::{Config, Role};

/// One entry as the page sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrationDto {
    pub id: String,
    pub label: String,
    pub blurb: String,
    pub kind: catalogue::Kind,
    pub base_url: Option<String>,
    pub serves: Vec<Role>,
    /// What it can be asked for, per role — `{"stt": [...], "llm": [...]}`.
    ///
    /// Grouped rather than a flat list so the page can fill one
    /// dropdown without filtering, and absent for a role it does not
    /// serve rather than empty.
    pub models: std::collections::BTreeMap<String, Vec<String>>,
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
        catalogue::KNOWN
            .iter()
            .map(|known| IntegrationDto {
                id: known.id.to_string(),
                label: known.label.to_string(),
                blurb: known.blurb.to_string(),
                kind: known.kind,
                base_url: known.base_url.map(Into::into),
                serves: known.serves.to_vec(),
                models: models_of(known),
                added: cfg.as_ref().is_some_and(|cfg| is_added(cfg, known.id)),
                secret_key: known.secret_key(),
            })
            .collect(),
    )
}

/// The models it offers, keyed by the role they are for.
fn models_of(known: &Known) -> std::collections::BTreeMap<String, Vec<String>> {
    known
        .serves
        .iter()
        .map(|role| {
            let name = match role {
                Role::Stt => "stt",
                Role::Llm => "llm",
                _ => "other",
            };
            (
                name.to_string(),
                known.models_for(*role).map(Into::into).collect(),
            )
        })
        .collect()
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

    fn groq() -> &'static Known {
        catalogue::find("groq").expect("in the catalogue")
    }

    #[test]
    fn a_provider_offers_its_models_grouped_by_role() {
        // One dropdown, filled without filtering — and absent for a
        // role it does not serve rather than empty, so the page can
        // tell "nothing for this" from "nothing yet".
        let models = models_of(groq());
        assert!(models["stt"].contains(&"whisper-large-v3-turbo".to_string()));
        assert!(models["llm"].contains(&"openai/gpt-oss-20b".to_string()));
        assert!(!models["stt"].contains(&"openai/gpt-oss-20b".to_string()));
    }

    #[test]
    fn a_service_offers_none() {
        let tado = catalogue::find("tado").expect("in the catalogue");
        assert!(models_of(tado).is_empty());
        assert!(tado.secret_key().is_none());
    }

    #[test]
    fn a_provider_counts_as_added_once_the_config_names_it() {
        let toml = concat!(
            "[[providers]]
",
            "name = \"groq\"
",
            "base_url = \"https://api.groq.com/openai/v1\"
",
        );
        assert!(is_added(&cfg(toml), "groq"));
        assert!(!is_added(&cfg(""), "groq"));
    }

    #[test]
    fn a_provider_with_no_key_yet_is_still_added() {
        // Otherwise adding one and being asked for its key would make
        // the row vanish the moment you looked away from it.
        let toml = concat!(
            "[[providers]]
",
            "name = \"groq\"
",
            "base_url = \"https://api.groq.com/openai/v1\"
",
        );
        assert!(is_added(&cfg(toml), "groq"));
    }

    #[test]
    fn tado_and_linear_are_read_from_their_own_sections() {
        assert!(is_added(
            &cfg("[presence]
enabled = true
[presence.tado]
"),
            "tado"
        ));
        assert!(!is_added(&cfg(""), "tado"));
        assert!(is_added(
            &cfg("[integrations.linear]
team = \"niles\"
"),
            "linear"
        ));
        assert!(!is_added(&cfg(""), "linear"));
    }
}

//! What Niles can actually be connected to: the base system it is set
//! up with, and the add-ons it can have.
//!
//! The page this feeds used to be a pair of text boxes: a name and a
//! URL. That let somebody type "chatgpt" and a chatgpt.com address and
//! get a provider that would never work, because whether Niles can talk
//! to a service is a question about the code, not about the config —
//! and it also asked for a base URL that Niles already knows, since
//! there is only one address Cerebras's API lives at.
//!
//! The route keeps its name, `/integrations`, which the app and plugin
//! docs already use; "add-on" is what the page calls them.
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
    pub needs: String,
    pub kind: catalogue::Kind,
    /// Whole rather than an id, so the page can head a group without a
    /// second request and without knowing the groups itself. Entries
    /// arrive in page order, so the first appearance of a group is
    /// where it goes.
    pub group: catalogue::GroupInfo,
    pub importance: catalogue::Importance,
    /// Part of the base system, set up under Settings → System rather
    /// than listed on the Add-ons page. Said outright so the page need
    /// not know which group that is.
    pub system: bool,
    /// The roles that use it when they name no provider — the one to
    /// show as chosen before anybody has chosen.
    pub default_for: Vec<Role>,
    pub icon: String,
    /// Every key it keeps, which removing it deletes.
    pub secret_keys: Vec<String>,
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
            .map(|known| dto_of(known, cfg.as_deref()))
            .collect(),
    )
}

fn dto_of(known: &Known, cfg: Option<&Config>) -> IntegrationDto {
    IntegrationDto {
        id: known.id.to_string(),
        label: known.label.to_string(),
        blurb: known.blurb.to_string(),
        needs: known.needs.to_string(),
        kind: known.kind,
        group: *known.group.info(),
        importance: known.importance,
        system: known.group == catalogue::Group::System,
        default_for: [Role::Stt, Role::Llm]
            .into_iter()
            .filter(|role| catalogue::default_provider(*role).id == known.id)
            .collect(),
        icon: known.icon.to_string(),
        secret_keys: known.secrets.iter().map(|k| k.to_string()).collect(),
        base_url: known.base_url.map(Into::into),
        serves: known.serves.to_vec(),
        models: models_of(known),
        added: cfg.is_some_and(|cfg| is_added(cfg, known.id)),
        secret_key: known.secret_key(),
    }
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
        "linear" => cfg.tool_servers.contains_key("linear"),
        // Switched off is still added: the card stays, with its switch,
        // its model and its token.
        "claude_code" => cfg.integrations.claude_code.is_some(),
        "chatgpt" => cfg.integrations.chatgpt.is_some(),
        "nemlig" => cfg.integrations.nemlig.is_some(),
        "spotify" => cfg.integrations.spotify.is_some(),
        "microsoft" => cfg.integrations.microsoft.is_some(),
        "google" => cfg.integrations.google.is_some(),
        // A host is what makes it a console; the section itself always
        // exists, with defaults, so its presence says nothing.
        "unifi" => cfg.presence.unifi.is_configured(),
        // Likewise an address: `[speakers]` always exists. Switched off
        // is still added, as with the others.
        "sonos" => !cfg.speakers.host.trim().is_empty(),
        // An address, as with UniFi and Sonos: `[tv]` always exists.
        "lg_tv" => !cfg.tv.host.trim().is_empty(),
        "mqtt" => !cfg.mqtt.host.trim().is_empty(),
        // An address again, whether the app or a ConfigMap wrote it.
        "web_search" => cfg.web_search.address().is_some(),
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
    fn a_provider_offers_its_models_grouped_by_role() {
        // One dropdown, filled without filtering — and absent for a
        // role it does not serve rather than empty, so the page can
        // tell "nothing for this" from "nothing yet".
        let cerebras = models_of(catalogue::find("cerebras").expect("in the catalogue"));
        assert!(cerebras["llm"].contains(&"gpt-oss-120b".to_string()));
        assert!(!cerebras.contains_key("stt"));
        let elevenlabs = models_of(catalogue::find("elevenlabs").expect("in the catalogue"));
        assert_eq!(elevenlabs["stt"], ["scribe_v2"]);
        assert!(!elevenlabs.contains_key("llm"));
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
            "name = \"cerebras\"
",
            "base_url = \"https://api.cerebras.ai/v1\"
",
        );
        assert!(is_added(&cfg(toml), "cerebras"));
        assert!(!is_added(&cfg(""), "cerebras"));
    }

    #[test]
    fn a_provider_with_no_key_yet_is_still_added() {
        // Otherwise adding one and being asked for its key would make
        // the row vanish the moment you looked away from it.
        let toml = concat!(
            "[[providers]]
",
            "name = \"cerebras\"
",
            "base_url = \"https://api.cerebras.ai/v1\"
",
        );
        assert!(is_added(&cfg(toml), "cerebras"));
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
            &cfg("[tool_servers.linear]
url = \"https://mcp.linear.app/mcp\"
"),
            "linear"
        ));
        assert!(!is_added(&cfg(""), "linear"));
        // The section from before Linear was a tool server is retired.
        assert!(!is_added(
            &cfg("[integrations.linear]
team = \"niles\"
"),
            "linear"
        ));
    }

    #[test]
    fn the_broker_counts_as_added_once_it_has_a_host() {
        // Not as a provider: "mqtt" is no `[[providers]]` entry, and
        // falling through to that arm would call it missing forever.
        assert!(is_added(&cfg("[mqtt]\nhost = \"10.0.0.2\"\n"), "mqtt"));
        assert!(!is_added(&cfg(""), "mqtt"));
    }

    #[test]
    fn web_search_is_added_by_its_address_from_the_app_or_a_config_map() {
        assert!(is_added(
            &cfg("[web_search]\nbase_url = \"https://search.example/search\"\n"),
            "web_search"
        ));
        assert!(!is_added(&cfg(""), "web_search"));
        // What removing it writes over an address a ConfigMap names.
        assert!(!is_added(
            &cfg("[web_search]\nbase_url = \"\"\n"),
            "web_search"
        ));
    }

    #[test]
    fn the_system_is_marked_so_the_add_ons_page_can_leave_it_out() {
        let system: Vec<String> = catalogue::KNOWN
            .iter()
            .map(|known| dto_of(known, None))
            .filter(|dto| dto.system)
            .map(|dto| dto.id)
            .collect();
        assert_eq!(system, ["mqtt", "elevenlabs", "cerebras"]);
    }

    #[test]
    fn each_role_says_which_provider_it_uses_by_default() {
        let dto = |id: &str| dto_of(catalogue::find(id).expect("in the catalogue"), None);
        assert_eq!(dto("elevenlabs").default_for, [Role::Stt]);
        assert_eq!(dto("cerebras").default_for, [Role::Llm]);
        assert!(dto("mqtt").default_for.is_empty());
    }
}

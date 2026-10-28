//! Setting credentials from the app.
//!
//! Three routes, and deliberately no fourth: there is no way to read a
//! secret back. The app needs to know *whether* one is set, never what
//! it is, and a route that returns it is a route that can be made to
//! return it to somebody else.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_config::catalogue;
use std::collections::HashMap;

type Failure = (StatusCode, String);

/// Where credentials typed into the app are kept.
///
/// A trait for the one reason that matters to the API key: making and
/// replacing it has to be provable without a database, and that is the
/// part that must not quietly stop working.
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    async fn load_all(&self) -> niles_config::Result<HashMap<String, String>>;
    async fn set(&self, key: &str, value: &str) -> niles_config::Result<()>;
    async fn clear(&self, key: &str) -> niles_config::Result<()>;
}

#[async_trait::async_trait]
impl SecretStore for niles_db::PostgresSecrets {
    async fn load_all(&self) -> niles_config::Result<HashMap<String, String>> {
        niles_db::PostgresSecrets::load_all(self).await
    }
    async fn set(&self, key: &str, value: &str) -> niles_config::Result<()> {
        niles_db::PostgresSecrets::set(self, key, value).await
    }
    async fn clear(&self, key: &str) -> niles_config::Result<()> {
        niles_db::PostgresSecrets::clear(self, key).await
    }
}

/// A store that is a map, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemorySecrets(pub std::sync::Mutex<HashMap<String, String>>);

#[cfg(test)]
#[async_trait::async_trait]
impl SecretStore for MemorySecrets {
    async fn load_all(&self) -> niles_config::Result<HashMap<String, String>> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
    async fn set(&self, key: &str, value: &str) -> niles_config::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.into(), value.into());
        Ok(())
    }
    async fn clear(&self, key: &str) -> niles_config::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        Ok(())
    }
}

/// Every credential Niles knows how to be given, and where each one is
/// currently coming from.
///
/// A fixed list rather than whatever happens to be stored: somebody
/// setting Niles up needs to see the thing they have *not* set yet,
/// which by definition is not in the database.
const KNOWN: &[(&str, &str)] = &[
    ("mqtt.username", "Zigbee2MQTT broker username"),
    ("mqtt.password", "Zigbee2MQTT broker password"),
    // Not a secret — it travels in the redirect URL the browser
    // follows — but it is a value Niles has to be given, and it comes
    // through the same resolution. Leaving it out meant sign-in could
    // not be finished from the app at all.
    //
    ("auth.github_client_id", "GitHub OAuth client ID"),
    ("auth.github_client_secret", "GitHub OAuth client secret"),
    ("auth.session_secret", "Session signing secret"),
    (
        "integrations.claude_code.oauth_token",
        "Claude Code token, from `claude setup-token`",
    ),
    // Written by signing in to ChatGPT from its card, never typed.
    // Listed so the card can tell it is signed in, and so removing it
    // deletes them.
    (
        "integrations.chatgpt.access_token",
        "ChatGPT sign-in: access token",
    ),
    (
        "integrations.chatgpt.refresh_token",
        "ChatGPT sign-in: refresh token",
    ),
    (
        "integrations.chatgpt.account_id",
        "ChatGPT sign-in: account",
    ),
    ("integrations.chatgpt.expires_at", "ChatGPT sign-in: expiry"),
    ("integrations.nemlig.username", "nemlig.com email"),
    ("integrations.nemlig.password", "nemlig.com password"),
    ("integrations.podcasts.access_key_id", "R2 access key ID"),
    (
        "integrations.podcasts.secret_access_key",
        "R2 secret access key",
    ),
    ("integrations.spotify.client_id", "Spotify app Client ID"),
    (
        "integrations.spotify.client_secret",
        "Spotify app Client secret",
    ),
    (
        "integrations.microsoft.client_id",
        "Microsoft app Application (client) ID",
    ),
    (
        "integrations.microsoft.client_secret",
        "Microsoft app Client secret",
    ),
    ("integrations.google.client_id", "Google OAuth Client ID"),
    (
        "integrations.google.client_secret",
        "Google OAuth Client secret",
    ),
    // Issued by the TV when it was paired; written by the pairing, and
    // listed so it can be cleared to pair again.
    ("tv.client_key", "LG TV pairing key"),
    // Read on every poll, so saving it takes hold without a restart —
    // which is what makes the phone-pairing button appear at all.
    ("presence.unifi.api_key", "UniFi console API key"),
    // Read on every webhook, so saving it is what turns the webhook on.
    (
        crate::webhook::LINEAR_WEBHOOK_SECRET,
        "Linear webhook signing secret",
    ),
];

#[derive(serde::Serialize)]
pub struct SecretDto {
    /// Owned rather than borrowed now that a provider's key is built
    /// from its name.
    pub key: String,
    pub label: String,
    /// What it is used against — `api.cerebras.ai`, the broker's
    /// host:port. Absent for Niles's own secrets, which are used
    /// against nothing.
    pub hint: Option<String>,
    /// Where it comes from: `environment`, `stored`, or `unset`.
    ///
    /// One field rather than two booleans the caller has to combine.
    /// The first version reported `set` and `stored` separately and got
    /// `set` wrong — it only ever looked in the store, so every
    /// credential fed by an environment variable read as unset, and the
    /// page offered to overwrite working credentials. A single source,
    /// resolved by the same code that reads the secret, cannot drift
    /// like that.
    pub source: niles_config::Source,
}

#[derive(serde::Serialize)]
pub struct SecretsReport {
    /// False when there is no database or no encryption key, in which
    /// case nothing here can be changed from the app.
    pub writable: bool,
    pub secrets: Vec<SecretDto>,
}

#[derive(serde::Deserialize)]
pub struct SetSecret {
    pub value: String,
}

/// `GET /secrets` — what is set, and what is missing.
pub async fn list_secrets(State(state): State<AppState>) -> Json<SecretsReport> {
    let cfg = state.config.as_ref().map(|c| c.current());

    // A row per provider Niles ships, alongside the fixed ones — added
    // or not, because Voice & AI shows the key field beside whichever
    // provider is picked, before the save that declares it.
    let mut secrets: Vec<SecretDto> = catalogue::KNOWN
        .iter()
        .filter(|known| known.kind == catalogue::Kind::Provider)
        .filter_map(|known| {
            let key = known.secret_key()?;
            Some(SecretDto {
                label: format!("{} API key", known.label),
                hint: known.base_url.and_then(host_of),
                source: match cfg.as_ref() {
                    Some(cfg) => cfg.secret_source(&key),
                    None if niles_config::secrets::get(&key).is_some() => {
                        niles_config::Source::Stored
                    }
                    None => niles_config::Source::Unset,
                },
                key,
            })
        })
        .collect();

    secrets.extend(KNOWN.iter().map(|(key, label)| SecretDto {
        key: (*key).to_string(),
        label: (*label).to_string(),
        hint: cfg.as_ref().and_then(|cfg| cfg.secret_hint(key)),
        // Asked of the config, which is the thing that knows
        // which environment variable each purpose reads. Without
        // a config store there is nothing to ask, and a
        // credential can only have come from the store.
        source: match cfg.as_ref() {
            Some(cfg) => cfg.secret_source(key),
            None if niles_config::secrets::get(key).is_some() => niles_config::Source::Stored,
            None => niles_config::Source::Unset,
        },
    }));

    // Each tool server's key: Linear's always, since its add-on is
    // listed whether or not it has been added, and every other one that
    // is configured.
    if let Some(cfg) = cfg.as_ref() {
        let mut ids: Vec<&str> = cfg.tool_servers.keys().map(String::as_str).collect();
        if !ids.contains(&"linear") {
            ids.push("linear");
        }
        secrets.extend(ids.into_iter().map(|id| {
            let key = niles_config::tool_servers::secret_key(id);
            let name = cfg
                .tool_servers
                .get(id)
                .map(|s| s.display_name(id).to_string())
                .unwrap_or_else(|| "Linear".into());
            SecretDto {
                label: format!("{name} API key"),
                hint: cfg.secret_hint(&key),
                source: cfg.secret_source(&key),
                key,
            }
        }));
    }

    // Each installed plugin's declared secrets: only ever stored, since
    // no environment variable is named for them.
    if let Some(plugins) = state.plugins.as_ref() {
        secrets.extend(
            plugins
                .secret_settings()
                .await
                .into_iter()
                .map(|(key, label)| SecretDto {
                    source: if niles_config::secrets::get(&key).is_some() {
                        niles_config::Source::Stored
                    } else {
                        niles_config::Source::Unset
                    },
                    key,
                    label,
                    hint: None,
                }),
        );
    }

    Json(SecretsReport {
        writable: state.secrets.is_some(),
        secrets,
    })
}

/// The host out of a base URL, for the label beside a provider's key.
fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url.split_once("://")?.1;
    let host = rest.split(['/', '?']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// `PUT /secrets/{key}` — save one.
pub async fn set_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<SetSecret>,
) -> Result<StatusCode, Failure> {
    let key = known(&state, &key).await?;
    let store = writable(&state)?;
    if body.value.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "an empty secret is not a secret — use DELETE to clear it".into(),
        ));
    }
    store
        .set(&key, &body.value)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not save it: {e}")))?;
    reload(&state).await;
    restart_its_plugin(&state, &key);
    reconnect_its_server(&state, &key);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /secrets/{key}` — forget one.
pub async fn clear_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, Failure> {
    let key = known(&state, &key).await?;
    let store = writable(&state)?;
    store
        .clear(&key)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not clear it: {e}")))?;
    reload(&state).await;
    restart_its_plugin(&state, &key);
    reconnect_its_server(&state, &key);
    Ok(StatusCode::NO_CONTENT)
}

/// Only keys Niles actually reads: the fixed ones, a key for each
/// provider it ships, and each installed plugin's declared secrets
/// (`plugins.<id>.<key>`). A typo must not fill the table with secrets
/// nothing will look for — including `provider.typo.api_key`, which
/// would look plausible and never be read.
async fn known(state: &AppState, key: &str) -> Result<String, Failure> {
    if KNOWN.iter().any(|(k, _)| *k == key) {
        return Ok(key.to_string());
    }
    // Any provider in the catalogue, declared or not: the key is typed
    // in beside the provider as it is picked, and the save that
    // declares it comes after.
    let a_provider = catalogue::KNOWN.iter().any(|known| {
        known.kind == catalogue::Kind::Provider && known.secret_key().as_deref() == Some(key)
    });
    if a_provider {
        return Ok(key.to_string());
    }
    // A tool server's key: Linear's before its table exists, since the
    // add-on may save the key first; any other once it is configured.
    if let Some(id) = niles_config::tool_servers::server_of_secret(key)
        && (id == "linear"
            || state
                .config
                .as_ref()
                .is_some_and(|c| c.current().tool_servers.contains_key(id)))
    {
        return Ok(key.to_string());
    }
    if key.starts_with("plugins.")
        && let Some(plugins) = state.plugins.as_ref()
        && plugins
            .secret_settings()
            .await
            .iter()
            .any(|(declared, _)| declared == key)
    {
        return Ok(key.to_string());
    }
    Err((
        StatusCode::NOT_FOUND,
        format!("{key} is not a secret Niles reads"),
    ))
}

/// A plugin reads its secrets when it starts, so one that changed
/// starts it again, as saving its settings does.
fn restart_its_plugin(state: &AppState, key: &str) {
    let plugin = key
        .strip_prefix("plugins.")
        .and_then(|rest| rest.split_once('.'))
        .map(|(id, _)| id);
    if let (Some(id), Some(plugins)) = (plugin, state.plugins.as_ref()) {
        plugins.restart(id);
    }
}

/// A server is connected with its key, so a new key connects it again.
fn reconnect_its_server(state: &AppState, key: &str) {
    if let (Some(id), Some(servers)) = (
        niles_config::tool_servers::server_of_secret(key),
        state.tool_servers.as_ref(),
    ) {
        servers.refresh(id);
    }
}

pub(crate) fn writable(state: &AppState) -> Result<&dyn SecretStore, Failure> {
    state.secrets.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "secrets cannot be saved without a database and an encryption key".into(),
    ))
}

/// Re-read everything into the in-process store, so the change applies
/// without a restart for anything that resolves a secret per use.
///
/// Whole reload rather than patching the one key: it is a handful of
/// short strings, and the alternative is two ways for the map to be
/// wrong.
pub(crate) async fn reload(state: &AppState) {
    let Some(store) = state.secrets.as_ref() else {
        return;
    };
    match store.load_all().await {
        Ok(values) => niles_config::secrets::load(values),
        Err(e) => tracing::error!("[secrets] saved, but the reload failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::plugins::tests::{Fake, send, state};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;

    fn put(key: &str) -> Request<Body> {
        Request::put(format!("/secrets/{key}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"value":"hunter2"}"#))
            .unwrap()
    }

    #[tokio::test]
    async fn a_plugins_declared_secret_is_accepted_and_others_are_not() {
        let app = || crate::router(state(Arc::default()));
        // Accepted as a key Niles reads; this test Niles has no store
        // to keep it in, which is the next thing it is told.
        let (status, body) = send(app(), put("plugins.spotprice.api_key")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
        for undeclared in [
            "plugins.spotprice.password",
            "plugins.other.api_key",
            "plugins.spotprice",
        ] {
            let (status, _) = send(app(), put(undeclared)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{undeclared}");
        }
        let delete = Request::delete("/secrets/plugins.other.api_key")
            .body(Body::empty())
            .unwrap();
        let (status, _) = send(app(), delete).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_plugins_secrets_are_listed_unset_until_saved() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            crate::router(state(fake)),
            Request::get("/secrets").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let report: serde_json::Value = serde_json::from_str(&body).unwrap();
        let secret = report["secrets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "plugins.spotprice.api_key")
            .expect("listed");
        assert_eq!(secret["label"], "Spot prices: API key");
        assert_eq!(secret["source"], "unset");
    }

    #[tokio::test]
    async fn a_shipped_providers_key_is_accepted_before_it_is_declared_and_groqs_is_not() {
        let app = || crate::router(state(Arc::default()));
        // Typed in beside the provider as it is picked; the save that
        // declares it comes after.
        for shipped in ["provider.elevenlabs.api_key", "provider.cerebras.api_key"] {
            let (status, body) = send(app(), put(shipped)).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{shipped}: {body}");
        }
        let (status, _) = send(app(), put("provider.groq.api_key")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn every_shipped_provider_has_a_key_row() {
        let (_, body) = send(
            crate::router(state(Arc::default())),
            Request::get("/secrets").body(Body::empty()).unwrap(),
        )
        .await;
        let report: serde_json::Value = serde_json::from_str(&body).unwrap();
        let keys: Vec<&str> = report["secrets"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["key"].as_str())
            .filter(|k| k.starts_with("provider."))
            .collect();
        assert_eq!(
            keys,
            ["provider.elevenlabs.api_key", "provider.cerebras.api_key"]
        );
    }
}

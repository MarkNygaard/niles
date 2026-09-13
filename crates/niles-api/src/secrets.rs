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

type Failure = (StatusCode, String);

/// Every credential Niles knows how to be given, and where each one is
/// currently coming from.
///
/// A fixed list rather than whatever happens to be stored: somebody
/// setting Niles up needs to see the thing they have *not* set yet,
/// which by definition is not in the database.
const KNOWN: &[(&str, &str)] = &[
    ("mqtt.username", "Zigbee2MQTT broker username"),
    ("mqtt.password", "Zigbee2MQTT broker password"),
    ("stt.api_key", "Speech-to-text API key"),
    ("llm.api_key", "LLM API key"),
    // Not a secret — it travels in the redirect URL the browser
    // follows — but it is a value Niles has to be given, and it comes
    // through the same resolution. Leaving it out meant sign-in could
    // not be finished from the app at all.
    ("auth.github_client_id", "GitHub OAuth client ID"),
    ("auth.github_client_secret", "GitHub OAuth client secret"),
    ("auth.session_secret", "Session signing secret"),
    (
        "auth.api_token",
        "API token for callers that are not browsers",
    ),
    ("integrations.linear.api_key", "Linear API key"),
];

#[derive(serde::Serialize)]
pub struct SecretDto {
    pub key: &'static str,
    pub label: &'static str,
    /// What it is used against — `api.groq.com`, the broker's
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
    Json(SecretsReport {
        writable: state.secrets.is_some(),
        secrets: KNOWN
            .iter()
            .map(|(key, label)| SecretDto {
                key,
                label,
                hint: cfg.as_ref().and_then(|cfg| cfg.secret_hint(key)),
                // Asked of the config, which is the thing that knows
                // which environment variable each purpose reads. Without
                // a config store there is nothing to ask, and a
                // credential can only have come from the store.
                source: match cfg.as_ref() {
                    Some(cfg) => cfg.secret_source(key),
                    None if niles_config::secrets::get(key).is_some() => {
                        niles_config::Source::Stored
                    }
                    None => niles_config::Source::Unset,
                },
            })
            .collect(),
    })
}

/// `PUT /secrets/{key}` — save one.
pub async fn set_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<SetSecret>,
) -> Result<StatusCode, Failure> {
    let key = known(&key)?;
    let store = writable(&state)?;
    if body.value.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "an empty secret is not a secret — use DELETE to clear it".into(),
        ));
    }
    store
        .set(key, &body.value)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not save it: {e}")))?;
    reload(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /secrets/{key}` — forget one.
pub async fn clear_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, Failure> {
    let key = known(&key)?;
    let store = writable(&state)?;
    store
        .clear(key)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not clear it: {e}")))?;
    reload(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Only keys Niles actually reads, so a typo cannot fill the table with
/// secrets nothing will ever look for.
fn known(key: &str) -> Result<&'static str, Failure> {
    KNOWN
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(k, _)| *k)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("{key} is not a secret Niles reads"),
            )
        })
}

fn writable(state: &AppState) -> Result<&niles_db::PostgresSecrets, Failure> {
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
async fn reload(state: &AppState) {
    let Some(store) = state.secrets.as_ref() else {
        return;
    };
    match store.load_all().await {
        Ok(values) => niles_config::secrets::load(values),
        Err(e) => tracing::error!("[secrets] saved, but the reload failed: {e}"),
    }
}

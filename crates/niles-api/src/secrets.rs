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
    // Not a secret — it travels in the redirect URL the browser
    // follows — but it is a value Niles has to be given, and it comes
    // through the same resolution. Leaving it out meant sign-in could
    // not be finished from the app at all.
    //
    ("auth.github_client_id", "GitHub OAuth client ID"),
    ("auth.github_client_secret", "GitHub OAuth client secret"),
    ("auth.session_secret", "Session signing secret"),
    (
        "auth.api_token",
        "API token for callers that are not browsers",
    ),
    ("integrations.linear.api_key", "Linear API key"),
    // Read on every poll, so saving it takes hold without a restart —
    // which is what makes the phone-pairing button appear at all.
    ("presence.unifi.api_key", "UniFi console API key"),
];

#[derive(serde::Serialize)]
pub struct SecretDto {
    /// Owned rather than borrowed now that a provider's key is built
    /// from its name.
    pub key: String,
    pub label: String,
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

    // A row per configured provider, alongside the fixed ones. The
    // list has to include what is *not* set yet, which rules out
    // reading it from the store — but which providers exist is a
    // config question, and the answer changes when somebody adds one.
    let mut secrets: Vec<SecretDto> = cfg
        .as_ref()
        .map(|cfg| {
            cfg.providers
                .iter()
                .map(|p| SecretDto {
                    key: p.secret_key(),
                    label: format!("{} API key", p.name),
                    hint: Some(host_of(&p.base_url).unwrap_or_else(|| p.base_url.clone())),
                    source: cfg.secret_source(&p.secret_key()),
                })
                .collect()
        })
        .unwrap_or_default();

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
    let key = known(&state, &key)?;
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
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /secrets/{key}` — forget one.
pub async fn clear_secret(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, Failure> {
    let key = known(&state, &key)?;
    let store = writable(&state)?;
    store
        .clear(&key)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not clear it: {e}")))?;
    reload(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Only keys Niles actually reads, so a typo cannot fill the table with
/// secrets nothing will ever look for.
/// Only keys Niles actually reads: the fixed ones, plus a key for each
/// provider the config declares. A typo must not fill the table with
/// secrets nothing will look for — including `provider.typo.api_key`,
/// which would look plausible and never be read.
fn known(state: &AppState, key: &str) -> Result<String, Failure> {
    if KNOWN.iter().any(|(k, _)| *k == key) {
        return Ok(key.to_string());
    }
    let declared = state
        .config
        .as_ref()
        .map(|c| c.current())
        .is_some_and(|cfg| cfg.providers.iter().any(|p| p.secret_key() == key));
    if declared {
        return Ok(key.to_string());
    }
    Err((
        StatusCode::NOT_FOUND,
        format!("{key} is not a secret Niles reads"),
    ))
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

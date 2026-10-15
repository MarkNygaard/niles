//! The API key: the bearer token scripts and other systems use.
//!
//! Made by Niles rather than invented by the household. Asking somebody
//! to run `openssl rand` and paste the result somewhere was the whole
//! of setting it up, and a random string nobody has to think of is
//! exactly what a machine is better at.
//!
//! Unlike every other secret this one can be read back, because the
//! point of it is to be copied into a script. Only by a signed-in
//! person, though: a request carrying the key itself is refused here,
//! so a key that leaks cannot be used to read its replacement or to
//! lock the household out by rotating it.

use crate::secrets::SecretStore;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use rand::TryRng;

type Failure = (StatusCode, String);

/// What the key is saved under in the secret store.
pub const KEY: &str = "auth.api_token";

/// A new key: 32 bytes from the operating system, as hex.
///
/// The OS generator rather than the thread-local one, because this is
/// a credential that lives for months and the cost is one syscall.
/// Hex, because it survives being pasted into a shell, a header, a URL
/// or a YAML file without quoting.
pub fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("the operating system's random number generator failed");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Make a key when Niles starts with somewhere to keep one and none is
/// set, so every install with a database has one from the first start.
///
/// Only after the stored secrets have been read — made over one that
/// could not be read, it would replace a key scripts already use.
pub async fn ensure(cfg: &niles_config::Config, store: &dyn SecretStore) {
    if cfg.auth.resolve_api_token().is_some() {
        return;
    }
    match keep_new(store).await {
        Ok(_) => tracing::info!("[auth] made an API key and kept it"),
        Err(e) => tracing::error!("[auth] could not keep a new API key: {e}"),
    }
}

/// Make a key, keep it, and put it in force.
async fn keep_new(store: &dyn SecretStore) -> niles_config::Result<String> {
    let token = generate();
    store.set(KEY, &token).await?;
    niles_config::secrets::load(store.load_all().await?);
    Ok(token)
}

/// Where the key in force comes from.
///
/// No separate "generated": once kept, a key Niles made and one
/// somebody saved are the same thing, and both can be shown and
/// replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Stored,
    /// Set by an environment variable, which Niles can read and cannot
    /// change, so it is neither shown nor replaced from here.
    Environment,
    None,
}

#[derive(Debug, serde::Serialize)]
pub struct Report {
    pub source: Source,
    /// Only for a stored key. One from the environment was put there by
    /// whoever runs the deployment, and they already have it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The variable it comes from, when it comes from one, so the page
    /// can say where to change it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
    /// False without a database and an encryption key: there is
    /// nowhere to keep a key, so none is made.
    pub writable: bool,
}

/// The request was made with the API key rather than by a person.
fn refuse_the_key_itself(headers: &HeaderMap) -> Result<(), Failure> {
    // Any bearer token, not just the current one: an old key presented
    // here is no more a person than the current one is.
    match crate::auth::presented_token(headers) {
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            "the API key cannot read or replace itself — sign in to do that".into(),
        )),
        None => Ok(()),
    }
}

fn report(state: &AppState) -> Report {
    let config = state.config.as_ref().map(|c| c.current());
    let source = match config.as_ref().map(|c| c.secret_source(KEY)) {
        Some(niles_config::Source::Environment) => Source::Environment,
        Some(niles_config::Source::Stored) => Source::Stored,
        // Without a config store there is no environment variable to
        // name, and a key can only have come from the store.
        None if niles_config::secrets::get(KEY).is_some() => Source::Stored,
        _ => Source::None,
    };
    Report {
        source,
        token: (source == Source::Stored)
            .then(|| niles_config::secrets::get(KEY))
            .flatten(),
        env_var: (source == Source::Environment)
            .then(|| config.as_ref().and_then(|c| c.secret_env_var(KEY)))
            .flatten(),
        writable: state.secrets.is_some(),
    }
}

/// `GET /auth/api-token` — the key, for a signed-in person to copy.
pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Report>, Failure> {
    refuse_the_key_itself(&headers)?;
    Ok(Json(report(&state)))
}

/// `POST /auth/api-token/rotate` — a new key; the old one stops working
/// with this response.
///
/// Also how an install whose key was never made gets one.
pub async fn rotate(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Report>, Failure> {
    refuse_the_key_itself(&headers)?;
    if report(&state).source == Source::Environment {
        return Err((
            StatusCode::CONFLICT,
            "the API key is set by an environment variable; change it there".into(),
        ));
    }
    let store = crate::secrets::writable(&state)?;
    keep_new(store).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("could not keep a new key: {e}"),
        )
    })?;
    tracing::info!("[auth] a new API key was made; the old one no longer works");
    Ok(Json(report(&state)))
}

/// Tests that put an API key in the process-wide secret store hold this,
/// so one test's key is not another's.
#[cfg(test)]
pub(crate) static STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn config(extra: &str) -> std::sync::Arc<niles_config::Config> {
        let toml = format!("{}\n{extra}\n", crate::config_tests::base_toml());
        niles_config::ConfigStore::from_str_in_memory(&toml)
            .expect("fixture is valid")
            .current()
    }

    #[test]
    fn a_key_is_long_random_and_safe_to_paste() {
        let a = generate();
        assert_eq!(a.len(), 64, "32 bytes as hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, generate());
    }

    #[tokio::test]
    async fn the_first_start_with_a_database_makes_a_key() {
        let _guard = STORE_LOCK.lock().await;
        niles_config::secrets::load(Default::default());
        let store = MemorySecrets::default();

        ensure(&config(""), &store).await;

        let kept = store.0.lock().unwrap().get(KEY).cloned().expect("kept");
        assert_eq!(kept.len(), 64);
        // And in force, without a restart.
        assert_eq!(config("").auth.resolve_api_token(), Some(kept));
    }

    #[tokio::test]
    async fn a_key_already_kept_is_not_replaced() {
        let _guard = STORE_LOCK.lock().await;
        let store = MemorySecrets::default();
        store
            .0
            .lock()
            .unwrap()
            .insert(KEY.into(), "scripts-use-this".into());
        niles_config::secrets::load(store.0.lock().unwrap().clone());

        ensure(&config(""), &store).await;

        assert_eq!(
            store.0.lock().unwrap().get(KEY).map(String::as_str),
            Some("scripts-use-this")
        );
    }

    #[tokio::test]
    async fn a_key_from_the_environment_wins_and_nothing_is_made() {
        let _guard = STORE_LOCK.lock().await;
        niles_config::secrets::load(Default::default());
        // SAFETY: a test-only variable name nothing else reads.
        unsafe { std::env::set_var("NILES_TEST_API_TOKEN_ENSURE", "from-the-environment") };
        let cfg = config("[auth]\napi_token_env = \"NILES_TEST_API_TOKEN_ENSURE\"");
        let store = MemorySecrets::default();

        ensure(&cfg, &store).await;

        assert!(store.0.lock().unwrap().is_empty());
        assert_eq!(
            cfg.auth.resolve_api_token().as_deref(),
            Some("from-the-environment")
        );
    }
}

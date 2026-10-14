//! Signing in to ChatGPT from the app.
//!
//! A paste-back flow (see `niles_llm::chatgpt`): Niles makes the sign-in
//! link, the person signs in in another tab, the page it lands on fails
//! to load, and they paste that page's address back here. The PKCE
//! verifier stays on this side the whole time, keyed by the sign-in's
//! `state`, so the browser never holds anything that could finish a
//! sign-in on its own.
//!
//! What comes back is kept in the secret store, under the keys
//! `ChatGptSignIn` names. From then on Niles alone renews it.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use niles_config::ChatGptSignIn;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Failure = (StatusCode, String);

/// Long enough to sign in and find the address bar on a phone.
const TTL: Duration = Duration::from_secs(15 * 60);

struct Pending {
    state: String,
    verifier: String,
    started: Instant,
}

/// Sign-ins started and not yet finished. In memory on purpose: one is
/// worth fifteen minutes, and a restart in the middle costs a retry.
#[derive(Default)]
pub struct Logins {
    pending: Mutex<Vec<Pending>>,
}

impl Logins {
    fn begin(&self, state: String, verifier: String) {
        let mut pending = self.lock();
        pending.retain(|p| p.started.elapsed() < TTL);
        pending.push(Pending {
            state,
            verifier,
            started: Instant::now(),
        });
    }

    /// The sign-in a paste finishes, spent whether or not the exchange
    /// works: a code is good once.
    ///
    /// By its state when the paste carries one. A bare code carries
    /// none, and finishes the newest — the one whose tab the person has
    /// just been in.
    fn finish(&self, state: Option<&str>) -> Option<Pending> {
        let mut pending = self.lock();
        pending.retain(|p| p.started.elapsed() < TTL);
        let index = match state {
            Some(state) => pending.iter().position(|p| p.state == state)?,
            None => pending.len().checked_sub(1)?,
        };
        Some(pending.remove(index))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Pending>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(serde::Serialize)]
pub struct Started {
    pub authorize_url: String,
}

#[derive(serde::Deserialize)]
pub struct Pasted {
    pub pasted: String,
}

#[derive(serde::Serialize)]
pub struct SignedIn {
    pub email: Option<String>,
}

#[derive(serde::Serialize)]
pub struct Status {
    pub signed_in: bool,
    pub email: Option<String>,
    /// When the access token runs out. Niles renews it before then; this
    /// is for someone wondering whether it does.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether a sign-in could be kept at all: there is nowhere to put
    /// one without a database and its secrets key.
    pub writable: bool,
}

/// `GET /integrations/chatgpt` — whether somebody is signed in, and who.
pub async fn status(State(state): State<AppState>) -> Json<Status> {
    let sign_in = ChatGptSignIn::stored();
    Json(Status {
        signed_in: sign_in.is_some(),
        email: sign_in
            .as_ref()
            .and_then(|s| niles_llm::chatgpt::email(&s.access_token)),
        expires_at: sign_in
            .as_ref()
            .and_then(|s| chrono::DateTime::from_timestamp(s.expires_at, 0)),
        writable: state.secrets.is_some(),
    })
}

/// `POST /integrations/chatgpt/login` — the page to sign in on.
pub async fn start(State(state): State<AppState>) -> Result<Json<Started>, Failure> {
    crate::secrets::writable(&state)?;
    let login = niles_llm::chatgpt::start_login();
    state.chatgpt_logins.begin(login.state, login.verifier);
    Ok(Json(Started {
        authorize_url: login.authorize_url,
    }))
}

/// `POST /integrations/chatgpt/login/complete` — finish with the address
/// the sign-in landed on.
pub async fn complete(
    State(state): State<AppState>,
    Json(body): Json<Pasted>,
) -> Result<Json<SignedIn>, Failure> {
    let store = crate::secrets::writable(&state)?;
    let refused = |reason: String| (StatusCode::BAD_REQUEST, reason);
    let given = niles_llm::chatgpt::state_in(&body.pasted);
    let pending = state
        .chatgpt_logins
        .finish(given.as_deref())
        .ok_or_else(|| {
            refused(
                "That sign-in has expired, or was not started here. Press Sign in with ChatGPT again."
                    .into(),
            )
        })?;
    let code = niles_llm::chatgpt::code_from_paste(&body.pasted, &pending.state)
        .map_err(|e| refused(e.to_string()))?;
    let tokens = niles_llm::chatgpt::exchange_code(
        &reqwest::Client::new(),
        niles_llm::chatgpt::TOKEN_URL,
        &code,
        &pending.verifier,
    )
    .await
    .map_err(|e| refused(e.to_string()))?;
    let account_id = niles_llm::chatgpt::account_id(&tokens.access_token).ok_or_else(|| {
        refused(
            "Signed in, but not to a ChatGPT account. Use the account that has the subscription."
                .into(),
        )
    })?;
    let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
        refused("OpenAI did not hand over a way to stay signed in. Try again.".into())
    })?;
    let email = tokens
        .id_token
        .as_deref()
        .and_then(niles_llm::chatgpt::email)
        .or_else(|| niles_llm::chatgpt::email(&tokens.access_token));
    let expires_in = i64::try_from(tokens.expires_in).unwrap_or(3600);
    let sign_in = ChatGptSignIn {
        access_token: tokens.access_token,
        refresh_token,
        account_id,
        expires_at: chrono::Utc::now().timestamp() + expires_in,
    };
    for (key, value) in sign_in.entries() {
        store
            .set(key, &value)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not keep it: {e}")))?;
    }
    crate::secrets::reload(&state).await;
    tracing::info!(
        "[chatgpt] signed in{}",
        email
            .as_deref()
            .map(|e| format!(" as {e}"))
            .unwrap_or_default()
    );
    Ok(Json(SignedIn { email }))
}

/// `DELETE /integrations/chatgpt/login` — sign out: the tokens go.
pub async fn sign_out(State(state): State<AppState>) -> Result<StatusCode, Failure> {
    let store = crate::secrets::writable(&state)?;
    for key in ChatGptSignIn::KEYS {
        store
            .clear(key)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not sign out: {e}")))?;
    }
    crate::secrets::reload(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_finishes_the_sign_in_its_state_names() {
        let logins = Logins::default();
        logins.begin("one".into(), "v1".into());
        logins.begin("two".into(), "v2".into());
        assert_eq!(logins.finish(Some("one")).unwrap().verifier, "v1");
        // Spent: a code is good once.
        assert!(logins.finish(Some("one")).is_none());
        assert!(logins.finish(Some("never-started")).is_none());
    }

    #[test]
    fn a_bare_code_finishes_the_newest() {
        let logins = Logins::default();
        logins.begin("old".into(), "v-old".into());
        logins.begin("new".into(), "v-new".into());
        assert_eq!(logins.finish(None).unwrap().verifier, "v-new");
        assert_eq!(logins.finish(None).unwrap().verifier, "v-old");
        assert!(logins.finish(None).is_none());
    }

    #[test]
    fn a_stale_sign_in_cannot_be_finished() {
        let logins = Logins::default();
        logins.begin("late".into(), "v".into());
        logins.lock()[0].started = Instant::now() - TTL - Duration::from_secs(1);
        assert!(logins.finish(Some("late")).is_none());
    }
}

//! Signing in with GitHub.
//!
//! GitHub is the front door, not the boundary — an account there is
//! free, so what it proves is only that somebody controls a particular
//! verified address. Whether *that* address may use Niles is
//! [`AuthConfig::person_for`]'s answer, read from the config in force
//! at the moment they arrive.
//!
//! - `GET /auth/github/start`    — redirects to GitHub
//! - `GET /auth/github/callback` — GitHub sends the browser back here

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde::Deserialize;

use super::flow::{self, safe_next};
use super::session::{self, Session};
use crate::state::AppState;

const AUTHORIZE: &str = "https://github.com/login/oauth/authorize";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
const API: &str = "https://api.github.com";

/// `read:user` would also work, but `user:email` is the narrower ask and
/// the addresses are the only thing this needs. No `read:org`: the
/// boundary is a list here, not a GitHub organisation, so asking to see
/// somebody's memberships would be requesting a permission we never use.
const SCOPES: &str = "user:email";

/// Every GitHub request needs one, and a descriptive one is good manners.
const USER_AGENT: &str = "niles";

#[derive(Debug, Deserialize)]
pub struct StartQuery {
    /// Where to land afterwards. Checked by [`safe_next`].
    next: Option<String>,
}

/// `GET /auth/github/start`
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StartQuery>,
) -> Response {
    let Some(config) = state.config.as_ref().map(|store| store.current()) else {
        return refuse(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles has no config store",
        );
    };
    if !config.auth.is_enabled() {
        return refuse(StatusCode::NOT_FOUND, "signing in is not switched on");
    }
    let client_id = match config.auth.resolve_client_id() {
        Ok(id) => id,
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let (token, binding) = state.attempts.begin(safe_next(query.next.as_deref()));
    let secure = is_secure(&headers);
    // No `redirect_uri`. GitHub then uses the callback URL registered
    // on the app itself, which is the one thing here that cannot be
    // wrong — whereas a URL built from `Host` and `X-Forwarded-Proto`
    // is only right if the ingress sets both, and gets refused with
    // `redirect_uri_mismatch` when it does not. One registered URL, one
    // source of truth, and one less thing that has to agree.
    let url = format!(
        "{AUTHORIZE}?client_id={}&scope={}&state={}",
        urlencode(&client_id),
        urlencode(SCOPES),
        urlencode(&token),
    );

    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, url),
            (header::SET_COOKIE, flow::binding_cookie(&binding, secure)),
        ],
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    /// GitHub sends this when somebody presses Cancel.
    error: Option<String>,
}

/// `GET /auth/github/callback`
pub async fn callback(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let secure = is_secure(&headers);
    let clear = flow::clear_binding_cookie(secure);

    let outcome = authenticate(&app, &headers, query).await;
    match outcome {
        Ok((session, next)) => {
            let secret = match app
                .config
                .as_ref()
                .map(|store| store.current())
                .ok_or_else(|| "no config store".to_string())
                .and_then(|c| c.auth.resolve_session_secret().map_err(|e| e.to_string()))
            {
                Ok(secret) => secret,
                Err(e) => return signed_out(&clear, &e),
            };
            let token = session::sign(&secret, &session);
            tracing::info!("{} signed in", session.email);
            // `AppendHeaders`, not an array: axum's `IntoResponseParts`
            // for an array *inserts*, so a second `Set-Cookie`
            // replaces the first. Written as an array, this sent only
            // the binding-clear cookie and dropped the session — the
            // sign-in succeeded, the browser came back with nothing,
            // and the sign-in page reappeared for ever.
            (
                StatusCode::SEE_OTHER,
                AppendHeaders([
                    (header::LOCATION, next),
                    (header::SET_COOKIE, session::set_cookie(&token, secure)),
                    (header::SET_COOKIE, clear),
                ]),
            )
                .into_response()
        }
        Err(reason) => {
            tracing::warn!("github sign-in refused: {reason}");
            signed_out(&clear, &reason)
        }
    }
}

/// The whole of the check, so the caller has one place to handle failure.
async fn authenticate(
    app: &AppState,
    headers: &HeaderMap,
    query: CallbackQuery,
) -> Result<(Session, String), String> {
    if let Some(error) = query.error {
        return Err(format!("GitHub returned {error}"));
    }
    let code = query.code.ok_or("GitHub sent no code")?;
    let state = query.state.ok_or("GitHub sent no state")?;

    // Half of what authenticates this request; the cookie is the other.
    let binding = cookie(headers, flow::BINDING_COOKIE);
    let pending = app
        .attempts
        .finish(&state, binding.as_deref())
        .ok_or("that sign-in did not start here, or has expired")?;

    let config = app
        .config
        .as_ref()
        .map(|store| store.current())
        .ok_or("this Niles has no config store")?;
    if !config.auth.is_enabled() {
        return Err("signing in is not switched on".into());
    }
    let client_id = config.auth.resolve_client_id().map_err(|e| e.to_string())?;
    let client_secret = config
        .auth
        .resolve_client_secret()
        .map_err(|e| e.to_string())?;

    let token = exchange(&client_id, &client_secret, &code).await?;
    let email = verified_email(&token).await?;

    // The boundary, read from the config in force *now* — which is what
    // makes taking somebody off the list take effect immediately.
    config.auth.person_for(&email).ok_or_else(|| {
        format!(
            "nobody here uses {email}, the address GitHub verified for that account. \
             Add it under Settings → People, or sign in with the account that owns an \
             address already listed."
        )
    })?;

    Ok((Session::new(email), pending.next.clone()))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error_description: Option<String>,
    error: Option<String>,
}

async fn exchange(client_id: &str, client_secret: &str, code: &str) -> Result<String, String> {
    let response: TokenResponse = reqwest::Client::new()
        .post(TOKEN)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, USER_AGENT)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
        ])
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub's answer was not the shape we expected: {e}"))?;

    response.access_token.ok_or_else(|| {
        response
            .error_description
            .or(response.error)
            .unwrap_or_else(|| "GitHub declined without saying why".into())
    })
}

#[derive(Deserialize)]
struct GithubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

/// The primary **verified** address, and nothing else.
///
/// Deliberately not `/user`'s `email` field: that is whatever the
/// account holder typed into their profile, and anybody can type yours.
/// `/user/emails` is GitHub vouching for it.
async fn verified_email(token: &str) -> Result<String, String> {
    let emails: Vec<GithubEmail> = reqwest::Client::new()
        .get(format!("{API}/user/emails"))
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, USER_AGENT)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub's answer was not the shape we expected: {e}"))?;

    pick_verified(&emails).ok_or_else(|| {
        "that GitHub account has no verified email address, so there is nothing here to \
         match it against"
            .to_string()
    })
}

/// Primary and verified for preference; otherwise any verified one — a
/// primary that is unverified is worth nothing, and somebody whose
/// listed address is a secondary should still get in.
fn pick_verified(emails: &[GithubEmail]) -> Option<String> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .or_else(|| emails.iter().find(|e| e.verified))
        .map(|e| e.email.trim().to_lowercase())
}

/// `GET /auth/signout`
pub async fn sign_out(headers: HeaderMap) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/".to_string()),
            (
                header::SET_COOKIE,
                session::clear_cookie(is_secure(&headers)),
            ),
        ],
    )
        .into_response()
}

/// Whether the browser reached us over HTTPS.
///
/// TLS terminates at the ingress, so the connection here is plain and
/// only `X-Forwarded-Proto` knows. This decides one thing: whether the
/// session cookie is marked `Secure`. An ingress that does not set the
/// header costs that flag — the cookie still works — rather than
/// breaking sign-in, which is why the redirect URL is not built from it.
fn is_secure(headers: &HeaderMap) -> bool {
    header_str(headers, "x-forwarded-proto")
        .map(|proto| proto.split(',').next().unwrap_or("").trim() == "https")
        .unwrap_or(false)
}

fn header_str(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    session::from_header(header_str(headers, header::COOKIE)?, name)
}

fn refuse(status: StatusCode, reason: &str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": reason }))).into_response()
}

/// Back to the sign-in page with the reason attached, and the binding
/// cookie cleared so a failed attempt leaves nothing behind.
fn signed_out(clear: &str, reason: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        AppendHeaders([
            (
                header::LOCATION,
                format!("/?sign_in_error={}", urlencode(reason)),
            ),
            (header::SET_COOKIE, clear.to_string()),
        ]),
    )
        .into_response()
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(address: &str, primary: bool, verified: bool) -> GithubEmail {
        GithubEmail {
            email: address.into(),
            primary,
            verified,
        }
    }

    #[test]
    fn a_response_setting_two_cookies_sends_both() {
        // axum's `IntoResponseParts` for an array *inserts*, so a second
        // `Set-Cookie` silently replaces the first. Written that way,
        // the callback sent only the binding-clear cookie and dropped
        // the session — sign-in succeeded and the browser came back
        // with nothing, over and over.
        let response = (
            StatusCode::SEE_OTHER,
            AppendHeaders([
                (header::LOCATION, "/".to_string()),
                (header::SET_COOKIE, session::set_cookie("token", true)),
                (header::SET_COOKIE, flow::clear_binding_cookie(true)),
            ]),
        )
            .into_response();

        let cookies: Vec<&str> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        assert_eq!(cookies.len(), 2, "both cookies must survive: {cookies:?}");
        assert!(
            cookies.iter().any(|c| c.starts_with(session::COOKIE)),
            "the session cookie is the one that matters: {cookies:?}"
        );
        assert!(
            cookies.iter().any(|c| c.starts_with(flow::BINDING_COOKIE)),
            "and the spent binding cookie is still cleared: {cookies:?}"
        );
    }

    #[test]
    fn an_array_would_have_dropped_one_which_is_why_it_is_not_used() {
        // Pinning the upstream behaviour this works around, so that a
        // future tidy-up back to an array fails here rather than in a
        // browser.
        let response = (
            StatusCode::SEE_OTHER,
            [
                (header::SET_COOKIE, "first=1".to_string()),
                (header::SET_COOKIE, "second=2".to_string()),
            ],
        )
            .into_response();
        assert_eq!(
            response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .count(),
            1,
            "if axum ever appends instead, this guard can go"
        );
    }

    #[test]
    fn only_a_verified_address_counts() {
        // A primary GitHub can't vouch for is worth less than a
        // secondary it can.
        let emails = [
            email("unverified@example.com", true, false),
            email("Secondary@Example.com", false, true),
        ];
        assert_eq!(
            pick_verified(&emails).as_deref(),
            Some("secondary@example.com")
        );
    }

    #[test]
    fn the_primary_verified_one_wins() {
        let emails = [
            email("other@example.com", false, true),
            email("mine@example.com", true, true),
        ];
        assert_eq!(pick_verified(&emails).as_deref(), Some("mine@example.com"));
    }

    #[test]
    fn nothing_verified_means_no_address_to_trust() {
        assert!(pick_verified(&[email("a@example.com", true, false)]).is_none());
    }

    #[test]
    fn addresses_are_lowercased_so_the_allowlist_need_not_guess() {
        assert_eq!(
            pick_verified(&[email("  Mark@Example.COM ", true, true)]).as_deref(),
            Some("mark@example.com")
        );
    }

    #[test]
    fn a_proxy_chain_is_read_from_the_left() {
        // `X-Forwarded-Proto: https, http` means the browser used HTTPS
        // and a hop after that did not.
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https, http".parse().unwrap());
        assert!(is_secure(&headers));
    }

    #[test]
    fn plain_http_is_the_default_rather_than_an_assumption() {
        assert!(!is_secure(&HeaderMap::new()));
    }

    #[test]
    fn urlencoding_escapes_what_would_break_a_query() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("plain-Text_1.0~"), "plain-Text_1.0~");
    }
}

//! Who may use this API.
//!
//! See ARCHITECTURE.md § Signing in. Two ways past this: a **cookie**,
//! for a person with a browser, and a **bearer token**, for the terminal
//! — `/logs` and `/devices` get read from one, which is why `/logs`
//! exists at all. The token is the operator and is not subject to the
//! allowlist, because it is not a person; it is also the way back in if
//! the list is ever emptied by some route other than the UI.
//!
//! Switched on by there being somebody on the list. An install with
//! nobody listed serves everything, because a login page nobody can
//! pass is not a safer state — see `niles_config::AuthConfig`.

pub mod flow;
pub mod github;
pub mod session;

#[cfg(test)]
mod gate_tests;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Paths that answer before this middleware, and why each has to.
///
/// Deliberately a list rather than a prefix: a prefix is how a route
/// added later ends up unprotected by accident.
fn is_exempt(path: &str) -> bool {
    matches!(
        path,
        // A liveness probe cannot hold a session.
        "/healthz"
        // The routes that exist to get a request past this. Each
        // authenticates itself: the callback by a single-use state plus
        // a cookie binding it to one browser, and `status` by having
        // nothing worth protecting.
        | "/auth/status"
        | "/auth/github/start"
        | "/auth/github/callback"
        | "/auth/signout"
        // Signed with an HMAC over its own body. A cookie would mean
        // nothing to Linear.
        | "/webhooks/linear"
    )
}

/// Whether this is a file out of the embedded UI bundle.
///
/// The shell and its assets are not the API — there is no data in
/// them — and the page cannot draw a sign-in screen without its own
/// JavaScript, which a browser requests with `Accept: */*`. So letting
/// navigations through is not enough on its own: it serves the page and
/// then refuses the script that would have drawn something on it.
#[cfg(feature = "ui")]
fn is_ui_asset(path: &str) -> bool {
    crate::web::contains(path)
}

/// Without the `ui` feature there is no bundle, and so nothing extra is
/// public.
#[cfg(not(feature = "ui"))]
fn is_ui_asset(_path: &str) -> bool {
    false
}

/// Whether this request is for a page rather than an API call.
///
/// A person whose session has expired should get the sign-in page, not
/// a JSON body; a script should get the status code, not HTML.
fn wants_html(request: &Request) -> bool {
    request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// The gate.
pub async fn require_sign_in(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(store) = state.config.as_ref() else {
        // No config store means no allowlist to enforce. Subcommands
        // that serve the device API without one are unchanged.
        return next.run(request).await;
    };
    let config = store.current();
    let path = request.uri().path();
    if !config.auth.is_enabled() || is_exempt(path) || is_ui_asset(path) {
        return next.run(request).await;
    }

    if let Some(token) = state.api_token.as_deref()
        && presented_token(&request).is_some_and(|given| constant_time_eq(given, token))
    {
        return next.run(request).await;
    }

    // The allowlist is read here, from the config in force now — which
    // is what makes removing somebody take effect on their next request
    // rather than whenever their cookie happens to expire.
    if let Some(email) = signed_in_as(request.headers(), &config)
        && config.auth.person_for(&email).is_some()
    {
        return next.run(request).await;
    }

    if wants_html(&request) {
        // The page itself is served to everyone; what it shows when
        // nobody is signed in is the sign-in screen. Redirecting here
        // would mean the UI could not ask who it is talking to.
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "sign in to use this" })),
    )
        .into_response()
}

/// `GET /auth/status` — what the page may know before anybody signs in.
///
/// Exempt from the gate on purpose: the UI has to be able to ask
/// whether it needs to show a sign-in button, and answering "sign in
/// first" to that question is a loop.
///
/// Says whether sign-in is on and who you are. Never who *else* is
/// allowed — that is the allowlist, and it is behind the gate with
/// everything else in `/config`.
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(config) = state.config.as_ref().map(|store| store.current()) else {
        return axum::Json(serde_json::json!({ "enabled": false, "signed_in_as": null }))
            .into_response();
    };
    let enabled = config.auth.is_enabled();
    let who = enabled
        .then(|| signed_in_as(&headers, &config))
        .flatten()
        .filter(|email| config.auth.person_for(email).is_some());

    axum::Json(serde_json::json!({
        "enabled": enabled,
        "signed_in_as": who,
    }))
    .into_response()
}

fn signed_in_as(headers: &HeaderMap, config: &niles_config::Config) -> Option<String> {
    let secret = config.auth.resolve_session_secret().ok()?;
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    let token = session::from_header(header, session::COOKIE)?;
    session::verify(&secret, &token).map(|s| s.email)
}

fn presented_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Comparing a secret byte by byte and returning early leaks how much of
/// a guess was right.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_routes_that_get_you_in_are_not_behind_the_gate() {
        // Otherwise signing in requires being signed in.
        for path in [
            "/healthz",
            "/auth/status",
            "/auth/github/start",
            "/auth/github/callback",
            "/auth/signout",
            "/webhooks/linear",
        ] {
            assert!(is_exempt(path), "{path} must answer before the gate");
        }
    }

    #[test]
    fn everything_worth_protecting_is_behind_it() {
        for path in [
            "/devices",
            "/rooms/kitchen",
            "/rooms/kitchen/ceiling",
            "/config",
            "/config/history",
            "/config/undo",
            "/logs",
            "/events/stream",
        ] {
            assert!(!is_exempt(path), "{path} must not be exempt");
        }
    }

    #[test]
    fn an_exempt_path_is_matched_whole_not_by_prefix() {
        // A prefix is how `/healthz/../config` or a route added under
        // `/auth/` later ends up unprotected without anybody noticing.
        assert!(!is_exempt("/healthz/../config"));
        assert!(!is_exempt("/auth/github/start/extra"));
        assert!(!is_exempt("/authx"));
    }

    #[test]
    fn a_token_comparison_does_not_return_early() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secret-longer"));
        assert!(!constant_time_eq("", "x"));
    }
}

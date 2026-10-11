//! A person's calendar connections, from their profile.
//!
//! Connecting is Microsoft's own sign-in. What stands in for a
//! signature on the way back is the same pair the GitHub sign-in uses
//! (see `auth::flow`): a single-use `state`, and a cookie binding it to
//! the browser that started — here scoped to `/calendars`. The person
//! who comes back must also be the one who left.

use crate::auth::flow::Attempts;
use crate::auth::github::{cookie, header_str, is_secure};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use niles_calendar::{Access, Calendars, MicrosoftApp};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const BINDING_COOKIE: &str = "niles_calendar";
/// As long as the GitHub sign-in's: enough to get through Microsoft's
/// account picker and a consent screen.
const TTL_SECS: u64 = 10 * 60;
const CALLBACK: &str = "/calendars/microsoft/callback";
/// Where the browser lands afterwards: the profile. The outcome goes in
/// the query before the hash — the app's routes are the hash, and a
/// query inside it would be read as part of the page's name.
const PROFILE: &str = "#/me";

type Failure = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Serialize)]
pub struct ConnectionDto {
    pub id: String,
    pub name: String,
    pub account: String,
    /// `read` or `write`.
    pub access: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CalendarsDto {
    /// Whether a calendar can be connected at all here.
    pub available: bool,
    /// Why not, when not.
    pub reason: Option<String>,
    pub connections: Vec<ConnectionDto>,
}

/// `GET /calendars` — the signed-in person's own connections.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CalendarsDto>, Failure> {
    let person = person(&state, &headers)?;
    let calendars = calendars(&state)?;
    let connections = calendars
        .connections(&person)
        .await
        .map_err(|e| failed(StatusCode::BAD_GATEWAY, e.to_string()))?
        .into_iter()
        .map(|c| ConnectionDto {
            id: c.id,
            name: c.name,
            account: c.account,
            access: c.access.as_str(),
        })
        .collect();
    let reason = app_keys(&state).err();
    Ok(Json(CalendarsDto {
        available: reason.is_none(),
        reason,
        connections,
    }))
}

#[derive(Debug, Deserialize)]
pub struct StartQuery {
    pub name: String,
    pub access: String,
    /// The connection this one takes the place of: the same account
    /// connected again to change its access.
    #[serde(default)]
    pub replace: Option<String>,
}

/// What survives the round trip to Microsoft, kept beside the `state`.
#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    person: String,
    name: String,
    access: Access,
    replace: Option<String>,
    redirect_uri: String,
}

/// `GET /calendars/microsoft/start?name=Work&access=read` — a link the
/// browser follows, not a call the page makes: it ends at Microsoft.
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StartQuery>,
) -> Response {
    let outcome = async {
        let person = person(&state, &headers).map_err(|(_, body)| reason_of(&body))?;
        calendars(&state).map_err(|(_, body)| reason_of(&body))?;
        let name = query.name.trim();
        if name.is_empty() || name.chars().count() > 40 {
            return Err("give the calendar a short name, like Private or Work".to_string());
        }
        let access =
            Access::parse(&query.access).ok_or_else(|| "access is read or write".to_string())?;
        let (client_id, client_secret) = app_keys(&state)?;
        let redirect_uri = redirect_uri(&state, &headers);
        let pending = Pending {
            person,
            name: name.to_string(),
            access,
            replace: query.replace.clone(),
            redirect_uri: redirect_uri.clone(),
        };
        let json = serde_json::to_string(&pending).map_err(|e| e.to_string())?;
        let (flow_state, binding) = attempts(&state).begin(json);
        let app = MicrosoftApp {
            client_id,
            client_secret,
            redirect_uri,
        };
        Ok((app.authorize_url(&flow_state, access), binding))
    }
    .await;
    let secure = is_secure(&headers);
    match outcome {
        Ok((to, binding)) => (
            StatusCode::SEE_OTHER,
            AppendHeaders([
                (header::LOCATION, to),
                (header::SET_COOKIE, binding_cookie(&binding, secure)),
            ]),
        )
            .into_response(),
        Err(reason) => back(
            &format!("calendar_error={}", urlencoding::encode(&reason)),
            secure,
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// `GET /calendars/microsoft/callback` — where Microsoft sends the
/// browser back. Always ends on the profile, saying how it went.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let secure = is_secure(&headers);
    match connect(&state, &headers, query).await {
        Ok(name) => back(
            &format!("calendar=connected&name={}", urlencoding::encode(&name)),
            secure,
        ),
        Err(reason) => {
            tracing::warn!("[calendars] connecting failed: {reason}");
            back(
                &format!("calendar_error={}", urlencoding::encode(&reason)),
                secure,
            )
        }
    }
}

async fn connect(
    state: &AppState,
    headers: &HeaderMap,
    query: CallbackQuery,
) -> Result<String, String> {
    if let Some(error) = query.error {
        // A work account whose organisation does not let its people
        // consent to outside apps lands here, with AADSTS65001 or
        // AADSTS90094 and a description saying so.
        let said = query.error_description.unwrap_or(error);
        return Err(said.lines().next().unwrap_or("").trim().to_string());
    }
    let (Some(code), Some(flow_state)) = (query.code, query.state) else {
        return Err("Microsoft sent nothing back".into());
    };
    let pending = attempts(state)
        .finish(&flow_state, cookie(headers, BINDING_COOKIE).as_deref())
        .ok_or("that sign-in expired or was started elsewhere; try again")?;
    let pending: Pending =
        serde_json::from_str(&pending.next).map_err(|_| "that sign-in was not a calendar's")?;
    let person = person(state, headers).map_err(|(_, body)| reason_of(&body))?;
    if person != pending.person {
        return Err("signed in as somebody else than when it started".into());
    }
    let calendars = calendars(state).map_err(|(_, body)| reason_of(&body))?;
    let (client_id, client_secret) = app_keys(state)?;
    let app = MicrosoftApp {
        client_id,
        client_secret,
        redirect_uri: pending.redirect_uri,
    };
    let http = reqwest::Client::new();
    let tokens = app
        .exchange(&http, &code, pending.access)
        .await
        .map_err(|e| e.to_string())?;
    let account = niles_calendar::graph::account(&http, &tokens.access_token)
        .await
        .map_err(|e| e.to_string())?;
    let made = calendars
        .connect(
            &person,
            &pending.name,
            pending.access,
            &account,
            tokens,
            pending.replace.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "[calendars] {person} connected {} ({}) with {} access",
        made.name,
        made.account,
        made.access.as_str()
    );
    Ok(made.name)
}

/// `DELETE /calendars/{id}` — forget one of your own, and its token.
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let person = person(&state, &headers)?;
    let gone = calendars(&state)?
        .disconnect(&person, &id)
        .await
        .map_err(|e| failed(StatusCode::BAD_GATEWAY, e.to_string()))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(failed(
            StatusCode::NOT_FOUND,
            "no such calendar of yours".into(),
        ))
    }
}

/// The signed-in person, by the speaker their sign-in names: the same
/// slug voice knows them by, which is what makes "my calendar" theirs
/// whichever way they ask.
fn person(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let store = state.config.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance has no people configured".into(),
        )
    })?;
    let config = store.current();
    let email = crate::auth::signed_in_as(headers, &config).ok_or_else(|| {
        failed(
            StatusCode::UNAUTHORIZED,
            "sign in to connect a calendar".into(),
        )
    })?;
    config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            failed(
                StatusCode::CONFLICT,
                "your sign-in is not linked to a voice yet, so Niles could not tell it is you \
                 asking about your calendar"
                    .into(),
            )
        })
}

fn calendars(state: &AppState) -> Result<&Arc<Calendars>, Failure> {
    state.calendars.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "calendars need the database and its secrets key".into(),
        )
    })
}

fn attempts(state: &AppState) -> &Attempts {
    &state.calendar_attempts
}

/// The app registration's keys, or why there are none.
fn app_keys(state: &AppState) -> Result<(String, String), String> {
    let config = state.config.as_ref().ok_or("no config store")?.current();
    let microsoft = config
        .integrations
        .microsoft
        .as_ref()
        .filter(|m| m.enabled)
        .ok_or("Microsoft calendars are not switched on under Settings → Integrations")?;
    microsoft.resolve_credentials().map_err(|_| {
        "the Microsoft app's client ID and secret are missing under Settings → Integrations"
            .to_string()
    })
}

/// The redirect URI registered on the app: the one the config names,
/// or this server's own address as the browser reached it.
fn redirect_uri(state: &AppState, headers: &HeaderMap) -> String {
    let configured = state.config.as_ref().and_then(|c| {
        c.current()
            .integrations
            .microsoft
            .as_ref()
            .and_then(|m| m.redirect_uri.clone())
            .filter(|u| !u.trim().is_empty())
    });
    if let Some(uri) = configured {
        return uri.trim().to_string();
    }
    let host = header_str(headers, "x-forwarded-host")
        .or_else(|| header_str(headers, header::HOST))
        .and_then(|h| h.split(',').next())
        .unwrap_or("localhost")
        .trim();
    let scheme = if is_secure(headers) { "https" } else { "http" };
    format!("{scheme}://{host}{CALLBACK}")
}

fn binding_cookie(value: &str, secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}={value}; Path=/calendars; HttpOnly; SameSite=Lax; Max-Age={TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn clear_binding_cookie(secure: bool) -> String {
    format!(
        "{BINDING_COOKIE}=; Path=/calendars; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// To the profile, with the outcome for it to say.
fn back(outcome: &str, secure: bool) -> Response {
    (
        StatusCode::SEE_OTHER,
        AppendHeaders([
            (header::LOCATION, format!("/?{outcome}{PROFILE}")),
            (header::SET_COOKIE, clear_binding_cookie(secure)),
        ]),
    )
        .into_response()
}

fn reason_of(body: &Json<serde_json::Value>) -> String {
    body.0
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("something went wrong")
        .to_string()
}

fn failed(status: StatusCode, message: String) -> Failure {
    (status, Json(serde_json::json!({ "error": message })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;

    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn the_redirect_is_the_address_the_browser_used() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "niles.mnygaard.io".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        );
        assert_eq!(
            redirect_uri(&state, &headers),
            "https://niles.mnygaard.io/calendars/microsoft/callback"
        );
    }

    #[test]
    fn the_binding_cookie_is_only_sent_back_to_calendars() {
        let set = binding_cookie("abc", true);
        assert!(set.contains("Path=/calendars;"));
        assert!(set.contains("HttpOnly"));
        assert!(set.ends_with("; Secure"));
    }
}

//! A person's calendar connections, from their profile.
//!
//! Microsoft and Google are connected through their own sign-in. What
//! stands in for a signature on the way back is the same pair the
//! GitHub sign-in uses (see `auth::flow`): a single-use `state`, and a
//! cookie binding it to the browser that started — here scoped to
//! `/calendars`. The person who comes back must also be the one who
//! left. iCloud has no sign-in to send anyone through: its Apple ID and
//! app-specific password are posted here, and checked with Apple before
//! they are kept.

use crate::auth::github::{cookie, header_str, is_secure};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use niles_calendar::{Access, Calendars, GoogleApp, Made, MicrosoftApp, Provider};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const BINDING_COOKIE: &str = "niles_calendar";
/// As long as the GitHub sign-in's: enough to get through an account
/// picker and a consent screen.
const TTL_SECS: u64 = 10 * 60;
/// Where the browser lands afterwards: the profile. The outcome goes in
/// the query before the hash — the app's routes are the hash, and a
/// query inside it would be read as part of the page's name.
const PROFILE: &str = "#/me";

type Failure = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Serialize)]
pub struct ConnectionDto {
    pub id: String,
    /// `microsoft`, `google` or `icloud`.
    pub provider: &'static str,
    pub name: String,
    pub account: String,
    /// `read` or `write`.
    pub access: &'static str,
    /// Whether the provider holds the connection to its access, or
    /// Niles does (iCloud).
    pub enforced_by_provider: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderDto {
    pub id: &'static str,
    /// Whether a calendar of this kind can be connected here.
    pub available: bool,
    /// Why not, when not.
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CalendarsDto {
    pub providers: Vec<ProviderDto>,
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
        .map(dto)
        .collect();
    let providers = [Provider::Microsoft, Provider::Google, Provider::Icloud]
        .into_iter()
        .map(|provider| {
            let reason = match provider {
                Provider::Icloud => None,
                oauth => keys(&state, oauth).err(),
            };
            ProviderDto {
                id: provider.as_str(),
                available: reason.is_none(),
                reason,
            }
        })
        .collect();
    Ok(Json(CalendarsDto {
        providers,
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

/// What survives the round trip to the provider, kept beside the
/// `state`.
#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    provider: Provider,
    person: String,
    name: String,
    access: Access,
    replace: Option<String>,
    redirect_uri: String,
}

/// `GET /calendars/{provider}/start?name=Work&access=read` — a link the
/// browser follows, not a call the page makes: it ends at Microsoft or
/// Google.
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<StartQuery>,
) -> Response {
    let outcome = async {
        let provider = oauth_provider(&provider)?;
        let person = person(&state, &headers).map_err(|(_, body)| reason_of(&body))?;
        calendars(&state).map_err(|(_, body)| reason_of(&body))?;
        let name = valid_name(&query.name)?;
        let access =
            Access::parse(&query.access).ok_or_else(|| "access is read or write".to_string())?;
        let (client_id, client_secret) = keys(&state, provider)?;
        let redirect_uri = redirect_uri(&state, &headers, provider);
        let pending = Pending {
            provider,
            person,
            name,
            access,
            replace: query.replace.clone(),
            redirect_uri: redirect_uri.clone(),
        };
        let json = serde_json::to_string(&pending).map_err(|e| e.to_string())?;
        let (flow_state, binding) = state.calendar_attempts.begin(json);
        let to = match provider {
            Provider::Google => GoogleApp {
                client_id,
                client_secret,
                redirect_uri,
            }
            .authorize_url(&flow_state, access),
            _ => MicrosoftApp {
                client_id,
                client_secret,
                redirect_uri,
            }
            .authorize_url(&flow_state, access),
        };
        Ok::<_, String>((to, binding))
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

/// `GET /calendars/{provider}/callback` — where Microsoft or Google
/// sends the browser back. Always ends on the profile, saying how it
/// went.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let secure = is_secure(&headers);
    match connect(&state, &headers, &provider, query).await {
        Ok(name) => back(
            &format!("calendar=connected&name={}", urlencoding::encode(&name)),
            secure,
        ),
        Err(reason) => {
            tracing::warn!("[calendars] connecting {provider} failed: {reason}");
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
    provider: &str,
    query: CallbackQuery,
) -> Result<String, String> {
    let provider = oauth_provider(provider)?;
    if let Some(error) = query.error {
        // A work account whose organisation does not let its people
        // consent to outside apps lands here, with AADSTS65001 or
        // AADSTS90094 and a description saying so; a Google account
        // that said no, with `access_denied`.
        let said = query.error_description.unwrap_or(error);
        return Err(said.lines().next().unwrap_or("").trim().to_string());
    }
    let (Some(code), Some(flow_state)) = (query.code, query.state) else {
        return Err("nothing came back from the sign-in".into());
    };
    let pending = state
        .calendar_attempts
        .finish(&flow_state, cookie(headers, BINDING_COOKIE).as_deref())
        .ok_or("that sign-in expired or was started elsewhere; try again")?;
    let pending: Pending =
        serde_json::from_str(&pending.next).map_err(|_| "that sign-in was not a calendar's")?;
    if pending.provider != provider {
        return Err("that sign-in was for another kind of calendar".into());
    }
    let person = person(state, headers).map_err(|(_, body)| reason_of(&body))?;
    if person != pending.person {
        return Err("signed in as somebody else than when it started".into());
    }
    let calendars = calendars(state).map_err(|(_, body)| reason_of(&body))?;
    let (client_id, client_secret) = keys(state, provider)?;
    let http = reqwest::Client::new();
    let (tokens, account) = match provider {
        Provider::Google => {
            let app = GoogleApp {
                client_id,
                client_secret,
                redirect_uri: pending.redirect_uri,
            };
            let tokens = app
                .exchange(&http, &code)
                .await
                .map_err(|e| e.to_string())?;
            let account = niles_calendar::google::account(&http, &tokens.access_token)
                .await
                .map_err(|e| e.to_string())?;
            (tokens, account)
        }
        _ => {
            let app = MicrosoftApp {
                client_id,
                client_secret,
                redirect_uri: pending.redirect_uri,
            };
            let tokens = app
                .exchange(&http, &code, pending.access)
                .await
                .map_err(|e| e.to_string())?;
            let account = niles_calendar::graph::account(&http, &tokens.access_token)
                .await
                .map_err(|e| e.to_string())?;
            (tokens, account)
        }
    };
    let credential = tokens
        .refresh_token
        .clone()
        .ok_or("no refresh token came back, so Niles could not keep the connection")?;
    let made = calendars
        .connect(
            &person,
            Made {
                provider,
                name: pending.name,
                access: pending.access,
                account,
                credential,
                endpoint: None,
                access_token: Some((tokens.access_token, tokens.expires_at)),
            },
            pending.replace.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "[calendars] {person} connected {} {} ({}) with {} access",
        provider.as_str(),
        made.name,
        made.account,
        made.access.as_str()
    );
    Ok(made.name)
}

#[derive(Debug, Deserialize)]
pub struct IcloudConnect {
    pub name: String,
    pub access: String,
    pub apple_id: String,
    pub password: String,
    #[serde(default)]
    pub replace: Option<String>,
}

/// `POST /calendars/icloud` — an Apple ID and an app-specific password,
/// kept only once Apple has accepted them.
pub async fn connect_icloud(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IcloudConnect>,
) -> Result<Json<ConnectionDto>, Failure> {
    let person = person(&state, &headers)?;
    let calendars = calendars(&state)?;
    let name = valid_name(&body.name).map_err(|e| failed(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    let access = Access::parse(&body.access).ok_or_else(|| {
        failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "access is read or write".into(),
        )
    })?;
    if body.apple_id.trim().is_empty() || body.password.trim().is_empty() {
        return Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the Apple ID and an app-specific password are both needed".into(),
        ));
    }
    let made = calendars
        .connect_icloud(
            &person,
            &name,
            access,
            &body.apple_id,
            &body.password,
            body.replace.as_deref(),
        )
        .await
        .map_err(|e| failed(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    tracing::info!(
        "[calendars] {person} connected icloud {} with {} access",
        made.name,
        made.access.as_str()
    );
    Ok(Json(dto(made)))
}

#[derive(Debug, Deserialize)]
pub struct AccessChange {
    pub access: String,
}

/// `PUT /calendars/{id}` — `{"access":"write"}`, for a connection whose
/// access Niles keeps (iCloud). The others sign in again.
pub async fn set_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AccessChange>,
) -> Result<Json<ConnectionDto>, Failure> {
    let person = person(&state, &headers)?;
    let access = Access::parse(&body.access).ok_or_else(|| {
        failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "access is read or write".into(),
        )
    })?;
    let changed = calendars(&state)?
        .set_access(&person, &id, access)
        .await
        .map_err(|e| failed(StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(dto(changed)))
}

/// `DELETE /calendars/{id}` — forget one of your own, and its
/// credential.
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

fn dto(c: niles_calendar::Connection) -> ConnectionDto {
    ConnectionDto {
        enforced_by_provider: c.provider.enforces_access(),
        id: c.id,
        provider: c.provider.as_str(),
        name: c.name,
        account: c.account,
        access: c.access.as_str(),
    }
}

fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("give the calendar a short name, like Private or Work".to_string());
    }
    Ok(name.to_string())
}

fn oauth_provider(value: &str) -> Result<Provider, String> {
    match Provider::parse(value) {
        Some(p @ (Provider::Microsoft | Provider::Google)) => Ok(p),
        _ => Err(format!("{value:?} is not a calendar Niles signs in to")),
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

/// A provider's app keys, or why there are none.
fn keys(state: &AppState, provider: Provider) -> Result<(String, String), String> {
    let config = state.config.as_ref().ok_or("no config store")?.current();
    let integrations = &config.integrations;
    match provider {
        Provider::Microsoft => integrations
            .microsoft
            .as_ref()
            .filter(|m| m.enabled)
            .ok_or("Microsoft calendars are not switched on under Settings → Integrations")?
            .resolve_credentials()
            .map_err(|_| {
                "the Microsoft app's client ID and secret are missing under Settings → Integrations"
                    .to_string()
            }),
        Provider::Google => integrations
            .google
            .as_ref()
            .filter(|g| g.enabled)
            .ok_or("Google calendars are not switched on under Settings → Integrations")?
            .resolve_credentials()
            .map_err(|_| {
                "the Google client ID and secret are missing under Settings → Integrations"
                    .to_string()
            }),
        Provider::Icloud => Err("iCloud has no app keys".into()),
    }
}

/// The redirect URI registered on the app: the one the config names,
/// or this server's own address as the browser reached it.
fn redirect_uri(state: &AppState, headers: &HeaderMap, provider: Provider) -> String {
    let configured = state.config.as_ref().and_then(|c| {
        let cfg = c.current();
        match provider {
            Provider::Google => cfg
                .integrations
                .google
                .as_ref()
                .and_then(|g| g.redirect_uri.clone()),
            _ => cfg
                .integrations
                .microsoft
                .as_ref()
                .and_then(|m| m.redirect_uri.clone()),
        }
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
    format!("{scheme}://{host}/calendars/{}/callback", provider.as_str())
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

    fn state() -> AppState {
        AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
    }

    #[test]
    fn the_redirect_is_the_address_the_browser_used() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "niles.mnygaard.io".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(
            redirect_uri(&state(), &headers, Provider::Microsoft),
            "https://niles.mnygaard.io/calendars/microsoft/callback"
        );
        assert_eq!(
            redirect_uri(&state(), &headers, Provider::Google),
            "https://niles.mnygaard.io/calendars/google/callback"
        );
    }

    #[test]
    fn icloud_is_not_signed_in_to() {
        assert!(oauth_provider("icloud").is_err());
        assert!(oauth_provider("yahoo").is_err());
        assert_eq!(oauth_provider("google"), Ok(Provider::Google));
    }

    #[test]
    fn the_binding_cookie_is_only_sent_back_to_calendars() {
        let set = binding_cookie("abc", true);
        assert!(set.contains("Path=/calendars;"));
        assert!(set.contains("HttpOnly"));
        assert!(set.ends_with("; Secure"));
    }
}

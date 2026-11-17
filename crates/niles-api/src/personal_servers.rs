//! My profile → MCP servers: each person's own remote MCP servers,
//! whose tools are theirs alone and only in their written chat.
//!
//! Every route answers the signed-in person only; another person's
//! server answers like one never kept. A key or token never goes back
//! out, and the browser sign-in's link and return never pass through the
//! chat.

use crate::auth::github::{header_str, is_secure};
use crate::state::AppState;
use crate::tool_servers::ServerStatus;
use async_trait::async_trait;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_mcp::{Key, Mode, NewServer, PersonalServer, ServerEdit, SignInKind};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

/// Where the browser comes back to after a server's own sign-in.
pub const CALLBACK: &str = "/me/tool-servers/callback";

/// Where the app shows the servers, after a sign-in.
const PAGE: &str = "/#/me/profile/mcp";

/// One of a person's servers as My profile shows it: never its key.
#[derive(Debug, Clone, Serialize)]
pub struct PersonalServerView {
    #[serde(flatten)]
    pub server: PersonalServer,
    pub status: ServerStatus,
    /// Why it is not working, in the server's words where it gave any.
    pub message: Option<String>,
    pub tools: Vec<PersonalToolView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonalToolView {
    /// The server's name for it, which is what its setting is keyed by.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub read_only: bool,
    pub destructive: bool,
    /// What it is set to now: the person's choice, or its default.
    pub mode: Mode,
}

/// What holds each person's connections. Implemented by the binary.
#[async_trait]
pub trait PersonalToolServers: Send + Sync {
    async fn list(&self, person: &str) -> Result<Vec<PersonalServerView>, niles_mcp::Error>;
    async fn add(
        &self,
        person: &str,
        server: NewServer,
    ) -> Result<PersonalServerView, niles_mcp::Error>;
    async fn update(
        &self,
        person: &str,
        id: i64,
        edit: ServerEdit,
    ) -> Result<Option<PersonalServerView>, niles_mcp::Error>;
    async fn remove(&self, person: &str, id: i64) -> Result<bool, niles_mcp::Error>;
    async fn set_mode(
        &self,
        person: &str,
        id: i64,
        tool: &str,
        mode: Mode,
    ) -> Result<bool, niles_mcp::Error>;
    /// The address the browser goes to, for a server that signs in
    /// there; `None` when the id isn't the person's.
    async fn begin_sign_in(
        &self,
        person: &str,
        id: i64,
        redirect_uri: &str,
    ) -> Result<Option<String>, niles_mcp::Error>;
    /// The browser came back with `code` for the sign-in `state` began.
    async fn finish_sign_in(
        &self,
        person: &str,
        state: &str,
        code: &str,
    ) -> Result<i64, niles_mcp::Error>;
    /// Connect again and list its tools afresh. Returns at once.
    async fn reconnect(&self, person: &str, id: i64) -> Result<bool, niles_mcp::Error>;
}

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, message.into())
}

fn servers(state: &AppState) -> Result<&dyn PersonalToolServers, Failure> {
    state.personal_servers.as_deref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "Your own MCP servers need Niles's database and its secrets key, and this Niles doesn't have both.",
        )
    })
}

/// The person signed in to this browser, keyed as their chat is. A
/// session only: the house's API key is nobody in particular.
fn signed_in_person(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let config = state
        .config
        .as_ref()
        .ok_or_else(|| {
            failed(
                StatusCode::NOT_IMPLEMENTED,
                "this Niles instance has no people configured",
            )
        })?
        .current();
    let email = crate::auth::signed_in_as(headers, &config)
        .ok_or_else(|| failed(StatusCode::UNAUTHORIZED, "sign in to add your own servers"))?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(niles_training::person_key(speaker.as_deref(), &email))
}

fn refused(e: niles_mcp::Error) -> Failure {
    match e {
        niles_mcp::Error::Invalid { kind, reason } => failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("the {kind}: {reason}"),
        ),
        niles_mcp::Error::NotPublic(reason) => failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("that address isn't allowed: {reason}"),
        ),
        why @ niles_mcp::Error::Storage(_) => {
            tracing::warn!("[my servers] {why}");
            failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "your servers couldn't be read or written",
            )
        }
        other => failed(StatusCode::BAD_GATEWAY, other.to_string()),
    }
}

const NO_SUCH: &str = "no such server";

/// What the add form posts. The key arrives as a [`Key`], which never
/// prints.
#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub name: String,
    pub url: String,
    pub sign_in: SignInKind,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub key: Option<Key>,
}

#[derive(Debug, Deserialize)]
pub struct EditBody {
    pub name: String,
    #[serde(default)]
    pub header: String,
    /// Absent or empty keeps the kept key.
    #[serde(default)]
    pub key: Option<Key>,
}

#[derive(Debug, Deserialize)]
pub struct ModeBody {
    pub mode: Mode,
}

fn read<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Failure> {
    // The fixed text never echoes the body: it may hold a key.
    body.map(|Json(b)| b).map_err(|_| {
        failed(
            StatusCode::BAD_REQUEST,
            "send the server as JSON with its name, address and how it signs in",
        )
    })
}

/// `GET /me/tool-servers`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PersonalServerView>>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    Ok(Json(servers(&state)?.list(&person).await.map_err(refused)?))
}

/// `POST /me/tool-servers`
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AddBody>, JsonRejection>,
) -> Result<(StatusCode, Json<PersonalServerView>), Failure> {
    let person = signed_in_person(&state, &headers)?;
    let servers = servers(&state)?;
    let b = read(body)?;
    let new = NewServer::new(&b.name, &b.url, b.sign_in, &b.header, b.key).map_err(refused)?;
    let view = servers.add(&person, new).await.map_err(refused)?;
    // The person key and the id only: never an address or a key.
    tracing::info!("[my servers] {person} added server {}", view.server.id);
    Ok((StatusCode::CREATED, Json(view)))
}

/// `PUT /me/tool-servers/{id}`
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Result<Json<EditBody>, JsonRejection>,
) -> Result<Json<PersonalServerView>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let servers = servers(&state)?;
    let b = read(body)?;
    let edit = ServerEdit::new(&b.name, &b.header, b.key).map_err(refused)?;
    servers
        .update(&person, id, edit)
        .await
        .map_err(refused)?
        .map(Json)
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, NO_SUCH))
}

/// `DELETE /me/tool-servers/{id}`
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    if !servers(&state)?
        .remove(&person, id)
        .await
        .map_err(refused)?
    {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    tracing::info!("[my servers] {person} removed server {id}");
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /me/tool-servers/{id}/tools/{tool}`
pub async fn set_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, tool)): Path<(i64, String)>,
    body: Result<Json<ModeBody>, JsonRejection>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let servers = servers(&state)?;
    let b = read(body)?;
    if !servers
        .set_mode(&person, id, &tool, b.mode)
        .await
        .map_err(refused)?
    {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/tool-servers/{id}/reconnect`
pub async fn reconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    if !servers(&state)?
        .reconnect(&person, id)
        .await
        .map_err(refused)?
    {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Serialize)]
pub struct SignInLink {
    /// Where the browser goes to sign in: the server's own page.
    pub url: String,
}

/// `POST /me/tool-servers/{id}/sign-in` — where the browser goes.
pub async fn sign_in(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<SignInLink>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let url = servers(&state)?
        .begin_sign_in(&person, id, &redirect_uri(&headers))
        .await
        .map_err(refused)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, NO_SUCH))?;
    Ok(Json(SignInLink { url }))
}

#[derive(Debug, Deserialize)]
pub struct Returned {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /me/tool-servers/callback` — the browser back from a server's
/// sign-in. Goes back to the page either way, saying how it went; the
/// code is used once and never shown.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(returned): Query<Returned>,
) -> Response {
    let outcome = finish(&state, &headers, returned).await;
    let to = match outcome {
        Ok(id) => format!("{PAGE}?signed_in={id}"),
        Err(why) => {
            tracing::info!("[my servers] a sign-in didn't finish: {why}");
            format!("{PAGE}?sign_in=failed")
        }
    };
    (StatusCode::SEE_OTHER, [(header::LOCATION, to)]).into_response()
}

async fn finish(state: &AppState, headers: &HeaderMap, returned: Returned) -> Result<i64, String> {
    let person = signed_in_person(state, headers).map_err(|(_, why)| why)?;
    if let Some(error) = returned.error {
        // The server's error code only: its description may quote what
        // was sent.
        return Err(format!(
            "the server said {}",
            error.chars().take(40).collect::<String>()
        ));
    }
    let (Some(code), Some(sign_in)) = (returned.code, returned.state) else {
        return Err("no code came back".into());
    };
    servers(state)
        .map_err(|(_, why)| why)?
        .finish_sign_in(&person, &sign_in, &code)
        .await
        .map_err(|e| e.to_string())
}

/// This Niles's own callback address, as the browser reached it: the
/// same rule as Strava's sign-in.
fn redirect_uri(headers: &HeaderMap) -> String {
    let host = header_str(headers, "x-forwarded-host")
        .or_else(|| header_str(headers, header::HOST))
        .and_then(|h| h.split(',').next())
        .unwrap_or("localhost")
        .trim();
    let scheme = if is_secure(headers) { "https" } else { "http" };
    format!("{scheme}://{host}{CALLBACK}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_mcp::{MemoryServers, PersonalServers};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_MY_SERVERS_SESSION_SECRET";
    const SECRET: &str = "the-my-servers-secret";
    const TOKEN_VAR: &str = "NILES_TEST_MY_SERVERS_API_TOKEN";
    const KEY: &str = "her-notion-key-0123456789";

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// The binary's part, over the store, with no connections.
    struct Fake(MemoryServers);

    fn view(server: PersonalServer) -> PersonalServerView {
        PersonalServerView {
            server,
            status: ServerStatus::Working,
            message: None,
            tools: Vec::new(),
        }
    }

    #[async_trait]
    impl PersonalToolServers for Fake {
        async fn list(&self, person: &str) -> Result<Vec<PersonalServerView>, niles_mcp::Error> {
            Ok(self.0.list(person).await?.into_iter().map(view).collect())
        }
        async fn add(
            &self,
            person: &str,
            server: NewServer,
        ) -> Result<PersonalServerView, niles_mcp::Error> {
            Ok(view(self.0.add(person, &server).await?))
        }
        async fn update(
            &self,
            person: &str,
            id: i64,
            edit: ServerEdit,
        ) -> Result<Option<PersonalServerView>, niles_mcp::Error> {
            Ok(self.0.update(person, id, &edit).await?.map(view))
        }
        async fn remove(&self, person: &str, id: i64) -> Result<bool, niles_mcp::Error> {
            self.0.remove(person, id).await
        }
        async fn set_mode(
            &self,
            person: &str,
            id: i64,
            tool: &str,
            mode: Mode,
        ) -> Result<bool, niles_mcp::Error> {
            self.0.set_mode(person, id, tool, mode).await
        }
        async fn begin_sign_in(
            &self,
            person: &str,
            id: i64,
            redirect_uri: &str,
        ) -> Result<Option<String>, niles_mcp::Error> {
            Ok(self
                .0
                .get(person, id)
                .await?
                .map(|_| format!("https://auth.example/authorize?redirect_uri={redirect_uri}")))
        }
        async fn finish_sign_in(
            &self,
            _person: &str,
            state: &str,
            _code: &str,
        ) -> Result<i64, niles_mcp::Error> {
            if state == "good" {
                Ok(1)
            } else {
                Err(niles_mcp::Error::SignIn(
                    "no sign-in of yours is waiting".into(),
                ))
            }
        }
        async fn reconnect(&self, person: &str, id: i64) -> Result<bool, niles_mcp::Error> {
            Ok(self.0.get(person, id).await?.is_some())
        }
    }

    fn app() -> axum::Router {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_MY_SERVERS_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_MY_SERVERS_CLIENT_SECRET", "secret");
            std::env::set_var(TOKEN_VAR, "the-house-token");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_MY_SERVERS_CLIENT_ID"
github_client_secret_env = "NILES_TEST_MY_SERVERS_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
api_token_env = "{TOKEN_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_personal_servers(Some(Arc::new(Fake(MemoryServers::new()))));
        crate::server::router(state)
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let cookie = format!(
            "{}={}",
            crate::auth::session::COOKIE,
            crate::auth::session::sign(SECRET, &crate::auth::session::Session::new(email, None))
        );
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::HOST, "niles.example")
            .header("x-forwarded-proto", "https")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String, Option<String>) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let location = res
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            String::from_utf8_lossy(&bytes).to_string(),
            location,
        )
    }

    fn notion(url: &str) -> Value {
        json!({ "name": "Notion", "url": url, "sign_in": "key", "key": KEY })
    }

    #[tokio::test]
    async fn a_server_is_its_persons_and_its_key_never_goes_back_out() {
        let app = app();
        let (status, added, _) = send(
            &app,
            by(
                "majse@example.com",
                "POST",
                "/me/tool-servers",
                Some(notion("https://mcp.notion.com/mcp")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{added}");
        assert!(!added.contains(KEY), "{added}");
        let id = serde_json::from_str::<Value>(&added).unwrap()["id"]
            .as_i64()
            .unwrap();

        let (_, hers, _) = send(
            &app,
            by("majse@example.com", "GET", "/me/tool-servers", None),
        )
        .await;
        assert!(hers.contains("Notion") && !hers.contains(KEY), "{hers}");
        let (_, his, _) = send(
            &app,
            by("mark@example.com", "GET", "/me/tool-servers", None),
        )
        .await;
        assert_eq!(his, "[]");

        for (method, uri, body) in [
            (
                "PUT",
                format!("/me/tool-servers/{id}"),
                Some(json!({ "name": "Mine now" })),
            ),
            ("DELETE", format!("/me/tool-servers/{id}"), None),
            (
                "PUT",
                format!("/me/tool-servers/{id}/tools/search"),
                Some(json!({ "mode": "on" })),
            ),
            ("POST", format!("/me/tool-servers/{id}/sign-in"), None),
            ("POST", format!("/me/tool-servers/{id}/reconnect"), None),
        ] {
            let (status, said, _) = send(&app, by("mark@example.com", method, &uri, body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {said}");
        }
    }

    #[tokio::test]
    async fn an_address_inside_is_refused_without_echoing_the_key() {
        let app = app();
        for inside in [
            "https://192.168.1.10/mcp",
            "https://niles.niles.svc.cluster.local/mcp",
            "http://mcp.notion.com/mcp",
        ] {
            let (status, said, _) = send(
                &app,
                by(
                    "majse@example.com",
                    "POST",
                    "/me/tool-servers",
                    Some(notion(inside)),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{inside}: {said}");
            assert!(!said.contains(KEY), "{said}");
        }
    }

    #[tokio::test]
    async fn the_sign_in_goes_out_with_niles_s_own_return_and_comes_back_to_the_page() {
        let app = app();
        let body =
            json!({ "name": "Linear", "url": "https://mcp.linear.app/mcp", "sign_in": "browser" });
        let (_, added, _) = send(
            &app,
            by("majse@example.com", "POST", "/me/tool-servers", Some(body)),
        )
        .await;
        let id = serde_json::from_str::<Value>(&added).unwrap()["id"]
            .as_i64()
            .unwrap();
        let (status, link, _) = send(
            &app,
            by(
                "majse@example.com",
                "POST",
                &format!("/me/tool-servers/{id}/sign-in"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{link}");
        assert!(
            link.contains("https://niles.example/me/tool-servers/callback"),
            "{link}"
        );

        let (status, _, to) = send(
            &app,
            by(
                "majse@example.com",
                "GET",
                "/me/tool-servers/callback?code=c&state=good",
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(to.as_deref(), Some("/#/me/profile/mcp?signed_in=1"));
        for query in ["code=c&state=stale", "error=access_denied&state=good", ""] {
            let (_, _, to) = send(
                &app,
                by(
                    "majse@example.com",
                    "GET",
                    &format!("/me/tool-servers/callback?{query}"),
                    None,
                ),
            )
            .await;
            assert_eq!(
                to.as_deref(),
                Some("/#/me/profile/mcp?sign_in=failed"),
                "{query}"
            );
        }
    }

    #[tokio::test]
    async fn the_house_token_is_nobody_s() {
        let app = app();
        let req = Request::builder()
            .uri("/me/tool-servers")
            .header(header::AUTHORIZATION, "Bearer the-house-token")
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

//! Third-party plugins: installing them from GitHub, updating, switching
//! off and removing them, how each is doing, and their pages and widgets.
//!
//! ```text
//! GET    /plugins                                            -> [PluginSummary]
//! POST   /plugins/install/preview   {repo, tag?}             -> the approval screen, with a token
//! POST   /plugins/install           {token, sha256?}         -> PluginSummary
//! POST   /plugins/{id}/check-update                          -> PluginSummary, with `update`
//! POST   /plugins/{id}/update/preview                        -> the approval screen, with the difference
//! POST   /plugins/{id}/update       {token, sha256?}         -> PluginSummary
//! POST   /plugins/{id}/rollback                              -> PluginSummary
//! POST   /plugins/{id}/enable                                -> PluginSummary
//! POST   /plugins/{id}/disable                               -> PluginSummary
//! DELETE /plugins/{id}                                       -> 204
//! GET    /plugins/{id}/logo                                  -> the sanitised SVG
//! GET    /plugins/{id}/surfaces/{surface}?room=kitchen       -> the node tree
//! POST   /plugins/{id}/surfaces/{surface}/actions/{control}  -> {toast, rerender}
//! ```
//!
//! An install is two requests so that what is installed is what was
//! shown: the preview downloads and checks the release and answers with
//! a token, and the install takes the token. A refusal is plain text,
//! with its status saying whose move it is: 400 a request that makes no
//! sense, 404 no such repo, release or plugin, 409 a clash with what is
//! installed, 410 a preview that expired, 422 a release that is not a
//! plugin this Niles runs, 502 GitHub, 503 no database.
//!
//! A page's tree is the shape `niles_plugins::Surface` documents. A
//! surface the plugin could not draw answers 422 with the plugin's own
//! sentence ("No prices yet."), one whose plugin is down 503 with its
//! status line, and the app shows either in place of the surface.
//!
//! The signed-in person reaches the plugin as their slug, never their
//! address: the speaker name their account is paired with, when it is.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

/// One installed plugin, for the Integrations page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    /// Where it appears on the Integrations page: a catalogue group, or
    /// `custom`.
    pub group: PluginGroup,
    /// `starting`, `running`, `not_set_up`, `failing`, `stopped` or
    /// `disabled`.
    pub status: String,
    /// The card's line: "Working", "Not working: timed out in on-timer
    /// after 10 s", "Off".
    pub status_line: String,
    /// The last thing that went wrong, if anything has, or what happened
    /// to it: an update that was rolled back.
    pub error: Option<String>,
    /// Events dropped because the plugin was not keeping up with them.
    pub dropped_events: u64,
    /// False once it is switched off.
    pub enabled: bool,
    /// `https://github.com/<owner>/<repo>`; none for one installed from
    /// a file.
    pub repo: Option<String>,
    pub release_tag: Option<String>,
    pub release_url: Option<String>,
    pub installed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Where its logo is served, when it has one.
    pub logo: Option<String>,
    /// The version a rollback goes back to, when there is one.
    pub previous_version: Option<String>,
    /// A newer release, when the last check found one.
    pub update: Option<PluginUpdate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginGroup {
    pub id: String,
    pub label: String,
}

/// A newer release, and what it would change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginUpdate {
    pub version: String,
    pub tag: String,
    pub release_url: String,
    pub notes: String,
    /// What it may do that the installed version may not, in plain words.
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// Why an install, an update or a change to a plugin did not happen.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginFailure {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    /// The release is not a plugin this Niles runs.
    Invalid(String),
    /// The preview's token expired or was used.
    Gone(String),
    /// GitHub did not answer, or refused.
    Upstream(String),
    /// No database, so nothing can be installed.
    Unavailable(String),
}

impl From<PluginFailure> for Failure {
    fn from(failure: PluginFailure) -> Self {
        match failure {
            PluginFailure::BadRequest(s) => (StatusCode::BAD_REQUEST, s),
            PluginFailure::NotFound(s) => (StatusCode::NOT_FOUND, s),
            PluginFailure::Conflict(s) => (StatusCode::CONFLICT, s),
            PluginFailure::Invalid(s) => (StatusCode::UNPROCESSABLE_ENTITY, s),
            PluginFailure::Gone(s) => (StatusCode::GONE, s),
            PluginFailure::Upstream(s) => (StatusCode::BAD_GATEWAY, s),
            PluginFailure::Unavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s),
        }
    }
}

/// Why a surface or an action got no answer.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceFailure {
    /// No such plugin or surface.
    NotFound(String),
    /// The request itself is wrong: an action value that is not one.
    BadRequest(String),
    /// The plugin's own error, or a tree Niles would not draw.
    Refused(String),
    /// The plugin is not running.
    Unavailable(String),
}

impl From<SurfaceFailure> for Failure {
    fn from(failure: SurfaceFailure) -> Self {
        match failure {
            SurfaceFailure::NotFound(s) => (StatusCode::NOT_FOUND, s),
            SurfaceFailure::BadRequest(s) => (StatusCode::BAD_REQUEST, s),
            SurfaceFailure::Refused(s) => (StatusCode::UNPROCESSABLE_ENTITY, s),
            SurfaceFailure::Unavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s),
        }
    }
}

/// Where a surface is drawn and for whom.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Viewer {
    /// The room whose card a widget is on.
    pub room: Option<String>,
    /// The signed-in person's slug.
    pub person: Option<String>,
}

/// What answers for the plugins. Implemented by the binary, which runs
/// them; previews and pages pass through as JSON so this crate needs
/// nothing of theirs.
#[async_trait]
pub trait Plugins: Send + Sync {
    async fn list(&self) -> Vec<PluginSummary>;
    async fn render(
        &self,
        id: &str,
        surface: &str,
        viewer: Viewer,
    ) -> Result<serde_json::Value, SurfaceFailure>;
    async fn act(
        &self,
        id: &str,
        surface: &str,
        control: &str,
        value: serde_json::Value,
        viewer: Viewer,
    ) -> Result<serde_json::Value, SurfaceFailure>;

    /// Download and check a release, and keep it under a token.
    async fn preview(
        &self,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<serde_json::Value, PluginFailure>;
    async fn install(
        &self,
        token: &str,
        sha256: Option<&str>,
    ) -> Result<PluginSummary, PluginFailure>;
    async fn check_update(&self, id: &str) -> Result<PluginSummary, PluginFailure>;
    async fn update_preview(&self, id: &str) -> Result<serde_json::Value, PluginFailure>;
    async fn update(
        &self,
        id: &str,
        token: &str,
        sha256: Option<&str>,
    ) -> Result<PluginSummary, PluginFailure>;
    async fn roll_back(&self, id: &str) -> Result<PluginSummary, PluginFailure>;
    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<PluginSummary, PluginFailure>;
    async fn remove(&self, id: &str) -> Result<(), PluginFailure>;
    /// The sanitised SVG, if it has one.
    async fn logo(&self, id: &str) -> Result<Option<String>, PluginFailure>;
    /// Every installed plugin's declared secrets, as
    /// `(plugins.<id>.<key>, label)`.
    async fn secret_settings(&self) -> Vec<(String, String)>;
    /// Start a fresh instance, as a change to its secrets needs.
    fn restart(&self, id: &str);
}

fn plugins(state: &AppState) -> Result<&dyn Plugins, Failure> {
    state.plugins.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles runs no plugins".to_string(),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct SurfaceQuery {
    #[serde(default)]
    pub room: Option<String>,
}

fn viewer(state: &AppState, headers: &HeaderMap, query: SurfaceQuery) -> Viewer {
    let person = state.config.as_ref().and_then(|store| {
        let config = store.current();
        let email = crate::auth::signed_in_as(headers, &config)?;
        config.auth.person_for(&email)?.speaker.clone()
    });
    Viewer {
        room: query.room.filter(|r| !r.trim().is_empty()),
        person,
    }
}

/// `GET /plugins`
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<PluginSummary>>, Failure> {
    Ok(Json(plugins(&state)?.list().await))
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub repo: String,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Approval {
    pub token: String,
    /// The component's sha256 as the preview showed it.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `POST /plugins/install/preview`
pub async fn preview(
    State(state): State<AppState>,
    Json(body): Json<PreviewRequest>,
) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(
        plugins(&state)?
            .preview(&body.repo, body.tag.as_deref())
            .await?,
    ))
}

/// `POST /plugins/install`
pub async fn install(
    State(state): State<AppState>,
    Json(body): Json<Approval>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(
        plugins(&state)?
            .install(&body.token, body.sha256.as_deref())
            .await?,
    ))
}

/// `POST /plugins/{id}/check-update`
pub async fn check_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(plugins(&state)?.check_update(&id).await?))
}

/// `POST /plugins/{id}/update/preview`
pub async fn update_preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Failure> {
    Ok(Json(plugins(&state)?.update_preview(&id).await?))
}

/// `POST /plugins/{id}/update`
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Approval>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(
        plugins(&state)?
            .update(&id, &body.token, body.sha256.as_deref())
            .await?,
    ))
}

/// `POST /plugins/{id}/rollback`
pub async fn roll_back(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(plugins(&state)?.roll_back(&id).await?))
}

/// `POST /plugins/{id}/enable`
pub async fn enable(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(plugins(&state)?.set_enabled(&id, true).await?))
}

/// `POST /plugins/{id}/disable`
pub async fn disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginSummary>, Failure> {
    Ok(Json(plugins(&state)?.set_enabled(&id, false).await?))
}

/// `DELETE /plugins/{id}`
pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    plugins(&state)?.remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /plugins/{id}/logo`
///
/// Served by Niles itself, so the Integrations page never fetches a
/// logo from anywhere else. The SVG was rewritten from an allowlist at
/// install; the headers are a second wall, in case a browser is ever
/// pointed at it directly: no scripts, nothing loaded, not sniffed as
/// anything but an image.
pub async fn logo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    let svg = plugins(&state)?
        .logo(&id)
        .await?
        .ok_or((StatusCode::NOT_FOUND, format!("{id} has no logo")))?;
    let mut response = svg.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // An update can change it; asking again is one small request.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}

/// `GET /plugins/{id}/surfaces/{surface}?room=kitchen`
pub async fn render(
    State(state): State<AppState>,
    Path((id, surface)): Path<(String, String)>,
    Query(query): Query<SurfaceQuery>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Failure> {
    let viewer = viewer(&state, &headers, query);
    Ok(Json(plugins(&state)?.render(&id, &surface, viewer).await?))
}

/// `POST /plugins/{id}/surfaces/{surface}/actions/{control}`, with the
/// value: `{"type":"pressed"}`, `{"type":"toggled","value":true}`,
/// `{"type":"number","value":21.5}`, `{"type":"chosen","value":"DK2"}`
/// or `{"type":"text","value":"hello"}`.
pub async fn act(
    State(state): State<AppState>,
    Path((id, surface, control)): Path<(String, String, String)>,
    Query(query): Query<SurfaceQuery>,
    headers: HeaderMap,
    Json(value): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, Failure> {
    let viewer = viewer(&state, &headers, query);
    Ok(Json(
        plugins(&state)?
            .act(&id, &surface, &control, value, viewer)
            .await?,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _: String, _: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    pub(crate) fn summary() -> PluginSummary {
        PluginSummary {
            id: "spotprice".into(),
            name: "Spot prices".into(),
            version: "0.1.0".into(),
            description: "Hear what power costs.".into(),
            authors: vec!["Jane".into()],
            homepage: None,
            group: PluginGroup {
                id: "custom".into(),
                label: "Custom".into(),
            },
            status: "failing".into(),
            status_line: "Not working: timed out in on-timer after 10 s".into(),
            error: Some("timed out in on-timer after 10 s".into()),
            dropped_events: 0,
            enabled: true,
            repo: Some("https://github.com/jane/niles-spotprice".into()),
            release_tag: Some("v0.1.0".into()),
            release_url: None,
            installed_at: None,
            updated_at: None,
            logo: Some("/plugins/spotprice/logo".into()),
            previous_version: None,
            update: None,
        }
    }

    /// A stand-in for the binary: one plugin, and every request it gets.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub seen: Mutex<Vec<(String, Viewer)>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn called(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }
    }

    #[async_trait]
    impl Plugins for Fake {
        async fn list(&self) -> Vec<PluginSummary> {
            vec![summary()]
        }

        async fn render(
            &self,
            id: &str,
            surface: &str,
            viewer: Viewer,
        ) -> Result<serde_json::Value, SurfaceFailure> {
            self.seen.lock().unwrap().push((surface.into(), viewer));
            match (id, surface) {
                ("spotprice", "electricity") => {
                    Ok(serde_json::json!({"nodes": [{"type": "stack", "children": []}]}))
                }
                ("spotprice", "empty") => Err(SurfaceFailure::Refused("No prices yet.".into())),
                _ => Err(SurfaceFailure::NotFound("no such page".into())),
            }
        }

        async fn act(
            &self,
            _: &str,
            _: &str,
            control: &str,
            value: serde_json::Value,
            _: Viewer,
        ) -> Result<serde_json::Value, SurfaceFailure> {
            match (control, value["type"].as_str()) {
                ("refresh", Some("pressed")) => {
                    Ok(serde_json::json!({"toast": "Prices updated", "rerender": true}))
                }
                _ => Err(SurfaceFailure::BadRequest("not an action value".into())),
            }
        }

        async fn preview(
            &self,
            repo: &str,
            tag: Option<&str>,
        ) -> Result<serde_json::Value, PluginFailure> {
            self.called(format!("preview {repo} {tag:?}"));
            match repo {
                "jane/niles-spotprice" => Ok(serde_json::json!({
                    "token": "t0k3n",
                    "permissions": ["Connect to api.energidataservice.dk"],
                })),
                "jane/private" => Err(PluginFailure::NotFound("no public repository".into())),
                _ => Err(PluginFailure::BadRequest("not a repo".into())),
            }
        }

        async fn install(
            &self,
            token: &str,
            sha256: Option<&str>,
        ) -> Result<PluginSummary, PluginFailure> {
            self.called(format!("install {token} {sha256:?}"));
            match token {
                "t0k3n" => Ok(summary()),
                _ => Err(PluginFailure::Gone("expired".into())),
            }
        }

        async fn check_update(&self, id: &str) -> Result<PluginSummary, PluginFailure> {
            self.called(format!("check {id}"));
            Ok(PluginSummary {
                update: Some(PluginUpdate {
                    version: "0.2.0".into(),
                    tag: "v0.2.0".into(),
                    release_url: String::new(),
                    notes: String::new(),
                    added: vec!["See button presses".into()],
                    removed: Vec::new(),
                }),
                ..summary()
            })
        }

        async fn update_preview(&self, id: &str) -> Result<serde_json::Value, PluginFailure> {
            self.called(format!("update preview {id}"));
            Ok(serde_json::json!({"token": "up", "added": ["See button presses"]}))
        }

        async fn update(
            &self,
            id: &str,
            token: &str,
            _: Option<&str>,
        ) -> Result<PluginSummary, PluginFailure> {
            self.called(format!("update {id} {token}"));
            Ok(summary())
        }

        async fn roll_back(&self, id: &str) -> Result<PluginSummary, PluginFailure> {
            self.called(format!("rollback {id}"));
            Err(PluginFailure::Conflict(
                "there is no earlier version".into(),
            ))
        }

        async fn set_enabled(
            &self,
            id: &str,
            enabled: bool,
        ) -> Result<PluginSummary, PluginFailure> {
            self.called(format!("enabled {id} {enabled}"));
            Ok(PluginSummary {
                enabled,
                ..summary()
            })
        }

        async fn remove(&self, id: &str) -> Result<(), PluginFailure> {
            self.called(format!("remove {id}"));
            match id {
                "spotprice" => Ok(()),
                _ => Err(PluginFailure::NotFound(format!("no plugin called {id}"))),
            }
        }

        async fn logo(&self, id: &str) -> Result<Option<String>, PluginFailure> {
            Ok((id == "spotprice").then(|| "<svg/>".to_string()))
        }

        async fn secret_settings(&self) -> Vec<(String, String)> {
            vec![(
                "plugins.spotprice.api_key".into(),
                "Spot prices: API key".into(),
            )]
        }

        fn restart(&self, id: &str) {
            self.called(format!("restart {id}"));
        }
    }

    pub(crate) fn state(fake: Arc<Fake>) -> AppState {
        AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher),
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_plugins(Some(fake))
    }

    fn app(fake: Arc<Fake>) -> axum::Router {
        crate::router(state(fake))
    }

    pub(crate) async fn send(app: axum::Router, request: Request<Body>) -> (StatusCode, String) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn get(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn the_list_carries_the_status_the_source_and_the_last_error() {
        let (status, body) = send(app(Arc::default()), get("/plugins")).await;
        assert_eq!(status, StatusCode::OK);
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["id"], "spotprice");
        assert_eq!(list[0]["status"], "failing");
        assert_eq!(list[0]["error"], "timed out in on-timer after 10 s");
        assert_eq!(list[0]["repo"], "https://github.com/jane/niles-spotprice");
        assert_eq!(list[0]["group"]["label"], "Custom");
        assert_eq!(list[0]["enabled"], true);
    }

    #[tokio::test]
    async fn a_preview_and_its_install_are_two_requests() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            post(
                "/plugins/install/preview",
                serde_json::json!({"repo": "jane/niles-spotprice"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("Connect to api.energidataservice.dk"));

        let (status, body) = send(
            app(fake.clone()),
            post(
                "/plugins/install",
                serde_json::json!({"token": "t0k3n", "sha256": "abc"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![
                "preview jane/niles-spotprice None",
                "install t0k3n Some(\"abc\")"
            ]
        );
    }

    #[tokio::test]
    async fn refusals_say_whose_move_it_is() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            post(
                "/plugins/install/preview",
                serde_json::json!({"repo": "jane/private"}),
            ),
        )
        .await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_FOUND, "no public repository")
        );
        let (status, _) = send(
            app(fake.clone()),
            post("/plugins/install", serde_json::json!({"token": "old"})),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let (status, _) = send(
            app(fake.clone()),
            post("/plugins/spotprice/rollback", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn updates_are_checked_previewed_and_applied() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            post("/plugins/spotprice/check-update", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let checked: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(checked["update"]["version"], "0.2.0");
        assert_eq!(checked["update"]["added"][0], "See button presses");

        let (status, _) = send(
            app(fake.clone()),
            post("/plugins/spotprice/update/preview", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            app(fake.clone()),
            post(
                "/plugins/spotprice/update",
                serde_json::json!({"token": "up"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            fake.calls.lock().unwrap()[1..],
            ["update preview spotprice", "update spotprice up"]
        );
    }

    #[tokio::test]
    async fn a_plugin_is_switched_off_on_and_removed() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            post("/plugins/spotprice/disable", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"enabled\":false"), "{body}");
        let (status, _) = send(
            app(fake.clone()),
            post("/plugins/spotprice/enable", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let delete = |id: &str| {
            Request::delete(format!("/plugins/{id}"))
                .body(Body::empty())
                .unwrap()
        };
        let (status, _) = send(app(fake.clone()), delete("spotprice")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(app(fake.clone()), delete("nope")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_logo_is_served_as_an_image_that_runs_nothing() {
        let response = app(Arc::default())
            .oneshot(get("/plugins/spotprice/logo"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let (status, _) = send(app(Arc::default()), get("/plugins/other/logo")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_surface_is_drawn_for_its_room() {
        let fake = Arc::new(Fake::default());
        let (status, body) = send(
            app(fake.clone()),
            get("/plugins/spotprice/surfaces/electricity?room=kitchen"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"stack\""));
        let seen = fake.seen.lock().unwrap();
        assert_eq!(seen[0].1.room.as_deref(), Some("kitchen"));
        assert_eq!(seen[0].1.person, None, "nobody is signed in");
    }

    #[tokio::test]
    async fn the_plugins_own_error_and_a_missing_page_are_told_apart() {
        let (status, body) = send(
            app(Arc::default()),
            get("/plugins/spotprice/surfaces/empty"),
        )
        .await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "No prices yet.")
        );
        let (status, _) = send(app(Arc::default()), get("/plugins/nope/surfaces/x")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_control_is_used() {
        let request = Request::post("/plugins/spotprice/surfaces/electricity/actions/refresh")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"type":"pressed"}"#))
            .unwrap();
        let (status, body) = send(app(Arc::default()), request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("Prices updated"));
    }

    #[tokio::test]
    async fn without_plugins_the_routes_say_so() {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher),
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        );
        let (status, _) = send(crate::router(state), get("/plugins")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}

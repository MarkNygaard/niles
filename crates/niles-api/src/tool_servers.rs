//! The remote tool servers: whether each is connected, and the tools it
//! offers. What each tool may do is config (`tool_servers.<id>.tools`),
//! written through `PATCH /config` like every other setting.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;

type Failure = (StatusCode, String);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    Connecting,
    /// Connected, and its tools listed.
    Working,
    /// No key, so nothing was tried.
    NeedsKey,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolServerView {
    pub id: String,
    pub status: ServerStatus,
    /// Why it is not working, in the server's words where it gave any.
    pub message: Option<String>,
    pub tools: Vec<RemoteToolView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteToolView {
    /// The server's name for it, which is what its setting is keyed by.
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub read_only: bool,
    pub destructive: bool,
}

/// What holds the connections. Implemented by the binary.
#[async_trait]
pub trait ToolServers: Send + Sync {
    async fn list(&self) -> Vec<ToolServerView>;
    /// Connect again and list its tools afresh. Returns at once; the
    /// list says when it is done.
    fn refresh(&self, id: &str);
}

fn servers(state: &AppState) -> Result<&dyn ToolServers, Failure> {
    state.tool_servers.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles connects to no tool servers".to_string(),
    ))
}

/// `GET /tool-servers`
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ToolServerView>>, Failure> {
    Ok(Json(servers(&state)?.list().await))
}

/// `POST /tool-servers/{id}/refresh`
pub async fn refresh(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let servers = servers(&state)?;
    let configured = state
        .config
        .as_ref()
        .is_some_and(|c| c.current().tool_servers.contains_key(&id));
    if !configured {
        return Err((StatusCode::NOT_FOUND, format!("no tool server {id}")));
    }
    servers.refresh(&id);
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::tests::send;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fake {
        refreshed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolServers for Fake {
        async fn list(&self) -> Vec<ToolServerView> {
            vec![ToolServerView {
                id: "linear".into(),
                status: ServerStatus::Failed,
                message: Some("401: the key was refused".into()),
                tools: vec![],
            }]
        }
        fn refresh(&self, id: &str) {
            self.refreshed.lock().unwrap().push(id.to_string());
        }
    }

    fn app(fake: Arc<Fake>) -> axum::Router {
        let toml = format!(
            "{}\n[tool_servers.linear]\nurl = \"https://mcp.linear.app/mcp\"\n",
            crate::config_tests::base_toml()
        );
        let store = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let state = crate::plugins::tests::state(Arc::default())
            .with_config_store(Some(store))
            .with_tool_servers(Some(fake as Arc<dyn ToolServers>));
        crate::router(state)
    }

    fn post(uri: &str) -> Request<Body> {
        Request::post(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn the_list_says_why_a_server_is_not_working() {
        let (status, body) = send(
            app(Arc::default()),
            Request::get("/tool-servers").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["status"], "failed");
        assert_eq!(list[0]["message"], "401: the key was refused");
    }

    #[tokio::test]
    async fn refreshes_a_configured_server_and_no_other() {
        let fake = Arc::new(Fake::default());
        let (status, _) = send(app(fake.clone()), post("/tool-servers/linear/refresh")).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, _) = send(app(fake.clone()), post("/tool-servers/github/refresh")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(*fake.refreshed.lock().unwrap(), ["linear"]);
    }

    fn put(key: &str) -> Request<Body> {
        Request::put(format!("/secrets/{key}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"value":"lin_api_x"}"#))
            .unwrap()
    }

    #[tokio::test]
    async fn a_configured_servers_key_is_one_niles_reads() {
        // Accepted as a key Niles reads; this Niles has nowhere to keep
        // it, which is what it says next.
        let (status, body) = send(app(Arc::default()), put("tool_servers.linear.api_key")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
        let (status, _) = send(app(Arc::default()), put("tool_servers.github.api_key")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

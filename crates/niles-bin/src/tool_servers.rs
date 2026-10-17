//! The remote tool servers, connected and kept in step with the config.
//!
//! Each `[tool_servers.<id>]` is one MCP session. Its tools go into the
//! shared [`DynamicTools`] as `<id>__<name>`, under the owner
//! `mcp:<id>`, beside the plugins'. A change to a server's address, key
//! or timeout connects it again; a change to which tools are on, its
//! keywords or its notes only offers its tools again, from the list it
//! already has.

use crate::approvals::{Approvals, AskFirst, ToolCard};
use async_trait::async_trait;
use niles_api::tool_servers::{RemoteToolView, ServerStatus, ToolServerView};
use niles_config::{ConfigStore, ToolMode, ToolServerConfig};
use niles_mcp::{McpClient, RemoteTool};
use niles_tools::{DynamicTools, Tool, ToolDescriptor};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// What the model reads back from one call, at most.
const MAX_RESULT: usize = 16 * 1024;
/// What a function name may be, at every provider Niles talks to.
const MAX_NAME: usize = 64;

pub(crate) struct ToolServers {
    settings: Arc<ConfigStore>,
    dynamic: DynamicTools,
    /// Where an Ask-first tool's call waits for its yes.
    approvals: Arc<Approvals>,
    servers: Mutex<BTreeMap<String, Server>>,
    refresh: mpsc::UnboundedSender<String>,
}

#[derive(Default)]
struct Server {
    status: Option<ServerStatus>,
    message: Option<String>,
    tools: Vec<RemoteTool>,
    client: Option<Arc<McpClient>>,
    /// What the live session was opened with.
    connected_with: Option<Connection>,
    /// Bumped by each attempt, so a slow one that finishes after a newer
    /// one has started is thrown away.
    attempt: u64,
}

#[derive(Clone, PartialEq)]
struct Connection {
    url: String,
    key: Option<String>,
    timeout_seconds: u64,
}

impl Connection {
    fn of(id: &str, cfg: &ToolServerConfig) -> Self {
        Self {
            url: cfg.url.trim().to_string(),
            key: cfg.api_key(id),
            timeout_seconds: cfg.timeout_seconds,
        }
    }
}

impl ToolServers {
    /// Connect every configured server, and follow the config from then
    /// on.
    pub(crate) fn start(
        settings: Arc<ConfigStore>,
        dynamic: DynamicTools,
        approvals: Arc<Approvals>,
    ) -> Arc<Self> {
        let (refresh, mut asked) = mpsc::unbounded_channel::<String>();
        let this = Arc::new(Self {
            settings,
            dynamic,
            approvals,
            servers: Mutex::new(BTreeMap::new()),
            refresh,
        });
        let task = this.clone();
        tokio::spawn(async move {
            let mut changes = task.settings.subscribe();
            let mut last = task.settings.current().tool_servers.clone();
            for id in last.keys() {
                task.sync(id, false);
            }
            loop {
                tokio::select! {
                    changed = changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        let now = task.settings.current().tool_servers.clone();
                        let ids: BTreeSet<&String> = last.keys().chain(now.keys()).collect();
                        for id in ids {
                            if last.get(id) != now.get(id) {
                                task.sync(id, false);
                            }
                        }
                        last = now;
                    }
                    Some(id) = asked.recv() => task.sync(&id, true),
                }
            }
        });
        this
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Server>> {
        self.servers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bring one server in line with its config. `reconnect` opens a new
    /// session even if nothing it was opened with has changed.
    fn sync(self: &Arc<Self>, id: &str, reconnect: bool) {
        let owner = owner(id);
        let Some(cfg) = self.settings.current().tool_servers.get(id).cloned() else {
            self.lock().remove(id);
            self.dynamic.remove(&owner);
            tracing::info!("[tool servers] {id} removed");
            return;
        };
        let wanted = Connection::of(id, &cfg);
        let attempt = {
            let mut servers = self.lock();
            let server = servers.entry(id.to_string()).or_default();
            if !reconnect
                && server.client.is_some()
                && server.connected_with.as_ref() == Some(&wanted)
            {
                let client = server.client.clone().expect("checked above");
                self.offer(id, &cfg, &client, &server.tools);
                return;
            }
            server.attempt += 1;
            server.client = None;
            server.connected_with = None;
            server.tools.clear();
            if wanted.key.is_none() {
                server.status = Some(ServerStatus::NeedsKey);
                server.message = Some("Needs its API key".into());
                self.dynamic.remove(&owner);
                return;
            }
            server.status = Some(ServerStatus::Connecting);
            server.message = None;
            server.attempt
        };
        self.dynamic.remove(&owner);

        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let key = wanted.key.clone().unwrap_or_default();
            let timeout = Duration::from_secs(wanted.timeout_seconds);
            let outcome = async {
                let (client, info) = McpClient::connect(&wanted.url, &key, timeout).await?;
                let tools = client.list_tools().await?;
                Ok::<_, niles_mcp::Error>((client, info, tools))
            }
            .await;

            let mut servers = this.lock();
            let Some(server) = servers.get_mut(&id).filter(|s| s.attempt == attempt) else {
                return;
            };
            match outcome {
                Ok((client, info, tools)) => {
                    let client = Arc::new(client);
                    tracing::info!(
                        "[tool servers] {id}: connected to {}, {} tools",
                        info.name.as_deref().unwrap_or(&wanted.url),
                        tools.len()
                    );
                    server.status = Some(ServerStatus::Working);
                    server.message = None;
                    server.tools = tools;
                    server.client = Some(client.clone());
                    server.connected_with = Some(wanted);
                    // Read again: the tools' settings may have changed
                    // while it connected.
                    if let Some(cfg) = this.settings.current().tool_servers.get(&id) {
                        this.offer(&id, cfg, &client, &server.tools);
                    }
                }
                Err(e) => {
                    tracing::warn!("[tool servers] {id}: {e}");
                    server.status = Some(ServerStatus::Failed);
                    server.message = Some(e.to_string());
                }
            }
        });
    }

    /// Put a server's tools that are on, or Ask first, into the
    /// registry.
    fn offer(
        &self,
        id: &str,
        cfg: &ToolServerConfig,
        client: &Arc<McpClient>,
        tools: &[RemoteTool],
    ) {
        let offered: Vec<Arc<dyn Tool>> = tools
            .iter()
            .filter(|tool| cfg.mode(&tool.name) != ToolMode::Off)
            .filter_map(|tool| {
                let descriptor = describe(id, tool).or_else(|| {
                    tracing::warn!(
                        "[tool servers] {id}: {:?} is not a name a model accepts; left out",
                        tool.name
                    );
                    None
                })?;
                let adapter = Arc::new(RemoteToolAdapter {
                    client: client.clone(),
                    name: tool.name.clone(),
                    descriptor,
                }) as Arc<dyn Tool>;
                if cfg.mode(&tool.name) == ToolMode::Ask {
                    let card = ToolCard {
                        title: format!("Let {} {}?", cfg.display_name(id), action(tool)),
                        subtitle: cfg.display_name(id).to_string(),
                        icon: Some(id.to_string()),
                        schema: tool.input_schema.clone(),
                    };
                    return Some(
                        Arc::new(AskFirst::new(adapter, self.approvals.clone(), card))
                            as Arc<dyn Tool>,
                    );
                }
                Some(adapter)
            })
            .collect();
        let notes = cfg.notes.trim();
        let notes = (!notes.is_empty()).then(|| {
            format!(
                "About the {} tools ({id}__…): {notes}",
                cfg.display_name(id)
            )
        });
        self.dynamic
            .set_with_notes(&owner(id), cfg.keywords_for(id), notes, offered);
    }
}

/// What a tool does, as the end of "Let Linear …?": its title, or its
/// name, in lower case.
fn action(tool: &RemoteTool) -> String {
    let words = tool
        .title
        .clone()
        .unwrap_or_else(|| tool.name.replace(['_', '-'], " "));
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => tool.name.clone(),
    }
}

fn owner(id: &str) -> String {
    format!("mcp:{id}")
}

/// One tool as the model sees it: its name prefixed with the server's
/// id, and its schema without the `$schema` some providers refuse.
fn describe(id: &str, tool: &RemoteTool) -> Option<ToolDescriptor> {
    let name = format!("{id}__{}", tool.name);
    let acceptable = name.len() <= MAX_NAME
        && !tool.name.is_empty()
        && tool
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !acceptable {
        return None;
    }
    let mut parameters = tool.input_schema.clone();
    if let Some(schema) = parameters.as_object_mut() {
        schema.remove("$schema");
        schema
            .entry("type")
            .or_insert_with(|| Value::String("object".into()));
    } else {
        parameters = serde_json::json!({ "type": "object" });
    }
    Some(ToolDescriptor {
        name,
        description: tool
            .title
            .as_deref()
            .filter(|_| tool.description.trim().is_empty())
            .unwrap_or(&tool.description)
            .to_string(),
        parameters,
    })
}

struct RemoteToolAdapter {
    client: Arc<McpClient>,
    /// The server's own name, which is what `tools/call` takes.
    name: String,
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for RemoteToolAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        match self.client.call_tool(&self.name, args).await {
            Ok(result) if !result.is_error => Ok(result_value(result.text)),
            Ok(result) if result.text.trim().is_empty() => {
                Err(niles_tools::Error::Remote(format!("{} failed", self.name)))
            }
            Ok(result) => Err(niles_tools::Error::Remote(cap(result.text))),
            Err(e) => Err(niles_tools::Error::Remote(format!(
                "{} could not be reached: {e}",
                self.descriptor.name
            ))),
        }
    }
}

/// Compact JSON as JSON, anything else as text, never more than 16 KiB.
fn result_value(text: String) -> Value {
    if text.len() <= MAX_RESULT
        && let Ok(value) = serde_json::from_str(&text)
    {
        return value;
    }
    Value::String(cap(text))
}

fn cap(text: String) -> String {
    if text.len() <= MAX_RESULT {
        return text;
    }
    let mut cut = MAX_RESULT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n[Cut off here: the result was longer than 16 KiB.]",
        &text[..cut]
    )
}

/// What the app is told.
pub(crate) struct AppToolServers(pub(crate) Arc<ToolServers>);

#[async_trait]
impl niles_api::tool_servers::ToolServers for AppToolServers {
    async fn list(&self) -> Vec<ToolServerView> {
        let configured = self.0.settings.current().tool_servers.clone();
        let servers = self.0.lock();
        configured
            .keys()
            .map(|id| {
                let server = servers.get(id);
                ToolServerView {
                    id: id.clone(),
                    status: server
                        .and_then(|s| s.status.clone())
                        .unwrap_or(ServerStatus::Connecting),
                    message: server.and_then(|s| s.message.clone()),
                    tools: server
                        .map(|s| {
                            s.tools
                                .iter()
                                .map(|t| RemoteToolView {
                                    name: t.name.clone(),
                                    title: t.title.clone(),
                                    description: t.description.clone(),
                                    read_only: t.read_only,
                                    destructive: t.destructive,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    fn refresh(&self, id: &str) {
        let _ = self.0.refresh.send(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// An MCP server with two tools, one of which deletes.
    async fn linear() -> MockServer {
        let server = MockServer::start().await;
        let answer = |result: Value| {
            move |req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].clone();
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            }
        };
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "initialize" })))
            .respond_with(answer(
                json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({ "method": "notifications/initialized" }),
            ))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(answer(json!({ "tools": [
                { "name": "list_issues", "description": "List issues.",
                  "inputSchema": { "$schema": "http://json-schema.org/draft-07/schema#", "type": "object" },
                  "annotations": { "readOnlyHint": true } },
                { "name": "delete_issue", "description": "Delete an issue.",
                  "annotations": { "destructiveHint": true } },
            ] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call" })))
            .respond_with(answer(
                json!({ "content": [{ "type": "text", "text": "{\"issues\":[]}" }] }),
            ))
            .mount(&server)
            .await;
        server
    }

    /// Change the config as the app does: an override, which is what
    /// removing the server can take away again.
    async fn write(settings: &ConfigStore, toml: &str) {
        let patch: toml::Table = toml::from_str(toml).unwrap();
        settings
            .apply(&patch, niles_config::ChangeSource::Api)
            .await
            .unwrap();
    }

    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out");
    }

    fn names(dynamic: &DynamicTools) -> Vec<String> {
        dynamic.names().into_iter().map(|(_, name)| name).collect()
    }

    // One test owns the environment variable, so tests that read it
    // cannot race each other.
    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "one tool server's life from connect to reset; each step depends on the one before"
    )]
    async fn offers_a_servers_tools_follows_its_settings_and_calls_them() {
        // SAFETY: only this test in the crate reads or writes it.
        unsafe { std::env::set_var("NILES_TEST_TOOL_SERVER_KEY", "lin_api_test") };
        let server = linear().await;
        let settings = Arc::new(ConfigStore::from_str_in_memory("").unwrap());
        let dynamic = DynamicTools::new();
        let servers = ToolServers::start(settings.clone(), dynamic.clone(), Arc::default());
        write(
            &settings,
            &format!(
                "[tool_servers.linear]\nurl = \"{}\"\napi_key_env = \"NILES_TEST_TOOL_SERVER_KEY\"\nnotes = \"Coding tasks go to Todo.\"\n",
                server.uri()
            ),
        )
        .await;

        until(|| names(&dynamic).len() == 2).await;
        let mut offered = names(&dynamic);
        offered.sort();
        assert_eq!(offered, ["linear__delete_issue", "linear__list_issues"]);
        let list = AppToolServers(servers.clone());
        let view = niles_api::tool_servers::ToolServers::list(&list).await;
        assert_eq!(view[0].status, ServerStatus::Working);
        assert!(
            view[0]
                .tools
                .iter()
                .any(|t| t.name == "delete_issue" && t.destructive)
        );

        // `$schema` is gone; the rest is the server's.
        let listed = dynamic.descriptors_of("mcp:linear");
        let list_issues = listed
            .iter()
            .find(|d| d.name == "linear__list_issues")
            .unwrap();
        assert_eq!(list_issues.parameters, json!({ "type": "object" }));

        // Switched off in the app: gone from the model, without a new
        // session.
        write(
            &settings,
            "[tool_servers.linear.tools]\ndelete_issue = \"off\"\n",
        )
        .await;
        until(|| names(&dynamic) == ["linear__list_issues"]).await;
        let initializes = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.body_json::<Value>().unwrap()["method"] == "initialize")
            .count();
        assert_eq!(initializes, 1);

        // A call goes to the server under its own name.
        let mut registry = niles_tools::ToolRegistry::new();
        registry.attach_dynamic(dynamic.clone());
        let result = registry
            .execute(&niles_llm::ToolCall {
                id: "1".into(),
                name: "linear__list_issues".into(),
                arguments: json!({}),
            })
            .await
            .unwrap();
        assert_eq!(result, json!({ "issues": [] }));
        assert_eq!(
            registry.notes_for("anything in linear"),
            ["About the linear tools (linear__…): Coding tasks go to Todo."]
        );

        // Ask first: still offered, but a call is held, not sent.
        write(
            &settings,
            "[tool_servers.linear.tools]\nlist_issues = \"ask\"\n",
        )
        .await;
        let asker = crate::approvals::Asker {
            person: "person:mark".into(),
            slug: Some("mark".into()),
            satellite: None,
            conversation: None,
        };
        let list = niles_llm::ToolCall {
            id: "2".into(),
            name: "linear__list_issues".into(),
            arguments: json!({ "team": "niles" }),
        };
        let call = || crate::approvals::ASKER.scope(asker.clone(), registry.execute(&list));
        let mut held = json!(null);
        for _ in 0..200 {
            held = call().await.unwrap();
            if held.get("waiting_for_approval").is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(held["shown_to_the_person"], "Let linear list issues?");
        let calls = || async {
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.body_json::<Value>().unwrap()["method"] == "tools/call")
                .count()
        };
        let before = calls().await;
        call().await.unwrap();
        assert_eq!(calls().await, before, "held, not sent");

        // Removed: every tool goes.
        settings
            .reset("tool_servers.linear", niles_config::ChangeSource::Api)
            .await
            .unwrap();
        until(|| names(&dynamic).is_empty()).await;
        unsafe { std::env::remove_var("NILES_TEST_TOOL_SERVER_KEY") };
    }

    #[test]
    fn leaves_out_a_name_no_model_accepts() {
        let tool = |name: &str| RemoteTool {
            name: name.into(),
            title: None,
            description: "x".into(),
            input_schema: json!({ "type": "object" }),
            read_only: false,
            destructive: false,
        };
        assert!(describe("linear", &tool("get_issue")).is_some());
        assert!(describe("linear", &tool("get issue")).is_none());
        assert!(describe("linear", &tool(&"x".repeat(60))).is_none());
    }

    #[test]
    fn cuts_a_long_result_at_16_kib() {
        let long = "é".repeat(MAX_RESULT);
        let Value::String(text) = result_value(long) else {
            panic!("text")
        };
        assert!(text.ends_with("[Cut off here: the result was longer than 16 KiB.]"));
    }
}

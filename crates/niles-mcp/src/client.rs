//! One MCP server, over Streamable HTTP.
//!
//! Every message is a POST. The server answers a request with either a
//! JSON body or a short `text/event-stream` that carries the reply and
//! then ends; both are read whole, which is all a client needs that
//! never opens the optional GET stream for server-initiated messages.

use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The revision Niles asks for. A server that speaks an older one says
/// so in its reply, and that is the one used after.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Pages of `tools/list` before giving up on a server that never stops
/// handing out cursors.
const MAX_PAGES: usize = 20;

/// What the server said about itself when the session began.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub name: Option<String>,
    pub instructions: Option<String>,
}

/// One tool, as the server describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// The server says it changes nothing.
    pub read_only: bool,
    /// The server says it may delete or overwrite. Only when it says so:
    /// MCP's default for an unannotated tool is "destructive", which
    /// would mark every tool of a server that annotates none.
    pub destructive: bool,
}

/// What a tool call produced: its text, and whether the server calls it
/// an error. A failed call is still an answer the model can read.
#[derive(Debug, Clone, PartialEq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

struct Session {
    id: Option<String>,
    protocol: String,
}

pub struct McpClient {
    http: reqwest::Client,
    url: String,
    key: String,
    session: Mutex<Session>,
    next_id: AtomicU64,
}

/// Why a POST did not produce a reply, where the difference matters.
enum Failure {
    /// The server no longer knows the session (`404` with a session id):
    /// start a new one and send again.
    SessionGone,
    Other(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Other(e)
    }
}

impl McpClient {
    /// Open a session: `initialize`, then `notifications/initialized`.
    pub async fn connect(url: &str, key: &str, timeout: Duration) -> Result<(Self, ServerInfo)> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let client = Self {
            http,
            url: url.to_string(),
            key: key.to_string(),
            session: Mutex::new(Session {
                id: None,
                protocol: PROTOCOL_VERSION.to_string(),
            }),
            next_id: AtomicU64::new(1),
        };
        let info = client.initialize().await?;
        Ok((client, info))
    }

    async fn initialize(&self) -> Result<ServerInfo> {
        {
            let mut session = self.lock();
            session.id = None;
            session.protocol = PROTOCOL_VERSION.to_string();
        }
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "niles", "version": env!("CARGO_PKG_VERSION") },
        });
        let result = match self.send_request("initialize", params).await {
            Ok(result) => result,
            Err(Failure::SessionGone) => {
                return Err(Error::Protocol(
                    "refused initialize as an unknown session".into(),
                ));
            }
            Err(Failure::Other(e)) => return Err(e),
        };
        if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
            self.lock().protocol = version.to_string();
        }
        self.notify("notifications/initialized").await?;
        Ok(ServerInfo {
            name: result
                .pointer("/serverInfo/name")
                .and_then(Value::as_str)
                .map(str::to_string),
            instructions: result
                .get("instructions")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Every tool the server offers, following its cursors.
    pub async fn list_tools(&self) -> Result<Vec<RemoteTool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Protocol("tools/list has no tools".into()))?;
            tools.extend(page.iter().filter_map(tool_of));
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        Err(Error::Protocol(format!(
            "tools/list was still paging after {MAX_PAGES} pages"
        )))
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallResult> {
        let arguments = if arguments.is_object() {
            arguments
        } else {
            json!({})
        };
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        Ok(call_result(&result))
    }

    /// A request, with one new session if the server has forgotten ours.
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        match self.send_request(method, params.clone()).await {
            Ok(result) => Ok(result),
            Err(Failure::SessionGone) => {
                tracing::info!("[mcp] {} forgot the session; starting a new one", self.url);
                self.initialize().await?;
                match self.send_request(method, params).await {
                    Ok(result) => Ok(result),
                    Err(Failure::SessionGone) => Err(Error::Protocol(
                        "the server forgot a session it had just started".into(),
                    )),
                    Err(Failure::Other(e)) => Err(e),
                }
            }
            Err(Failure::Other(e)) => Err(e),
        }
    }

    async fn send_request(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, Failure> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = self.post(&body).await?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.lock().id = Some(session.to_string());
        }
        let text = response
            .text()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let reply = if content_type.starts_with("text/event-stream") {
            sse_messages(&text)
                .into_iter()
                .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
                .ok_or_else(|| {
                    Error::Protocol(format!("no reply to {method} in the event stream"))
                })?
        } else {
            serde_json::from_str::<Value>(&text)
                .map_err(|e| Error::Protocol(format!("{method}: {e}")))?
        };
        if let Some(error) = reply.get("error") {
            return Err(Error::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message")
                    .to_string(),
            }
            .into());
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("{method}: a reply with no result")).into())
    }

    async fn notify(&self, method: &str) -> Result<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method });
        match self.post(&body).await {
            Ok(_) => Ok(()),
            Err(Failure::SessionGone) => Err(Error::Protocol(format!(
                "{method}: the session was refused"
            ))),
            Err(Failure::Other(e)) => Err(e),
        }
    }

    async fn post(&self, body: &Value) -> std::result::Result<reqwest::Response, Failure> {
        let (session, protocol) = {
            let s = self.lock();
            (s.id.clone(), s.protocol.clone())
        };
        let mut request = self
            .http
            .post(&self.url)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(body);
        if !self.key.is_empty() {
            request = request.bearer_auth(&self.key);
        }
        if let Some(session) = &session {
            request = request
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", protocol);
        }
        let response = request
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session.is_some() {
            return Err(Failure::SessionGone);
        }
        let reason = response.text().await.unwrap_or_default();
        Err(Error::Refused {
            status: status.as_u16(),
            reason: reason_of(status, &reason),
        }
        .into())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The body's own words when they are short enough to show, else what
/// the status means.
fn reason_of(status: reqwest::StatusCode, body: &str) -> String {
    let body = body.trim();
    let said = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("error_description"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| (!body.is_empty() && body.len() <= 200).then(|| body.to_string()));
    match (said, status.as_u16()) {
        (Some(said), _) => said,
        (None, 401) => "the key was refused".into(),
        (None, 403) => "the key is not allowed to do that".into(),
        (None, _) => status
            .canonical_reason()
            .unwrap_or("an error")
            .to_ascii_lowercase(),
    }
}

/// The JSON messages in an event stream, in order: the `data:` lines of
/// each event joined, as the SSE format puts them back together.
fn sse_messages(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    let mut flush = |data: &mut String| {
        if !data.is_empty() {
            if let Ok(message) = serde_json::from_str(data) {
                out.push(message);
            }
            data.clear();
        }
    };
    for line in body.lines() {
        if line.is_empty() {
            flush(&mut data);
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data);
    out
}

fn tool_of(value: &Value) -> Option<RemoteTool> {
    let name = value.get("name")?.as_str()?.to_string();
    let annotations = value.get("annotations");
    let hint = |key: &str| {
        annotations
            .and_then(|a| a.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let read_only = hint("readOnlyHint");
    Some(RemoteTool {
        title: value
            .get("title")
            .or_else(|| annotations.and_then(|a| a.get("title")))
            .and_then(Value::as_str)
            .map(str::to_string),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        input_schema: value
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" })),
        read_only,
        destructive: !read_only && hint("destructiveHint"),
        name,
    })
}

/// The text of a `tools/call` result: its text parts, or the structured
/// content when there are none, with anything else named rather than
/// dropped.
fn call_result(result: &Value) -> CallResult {
    let mut parts: Vec<String> = Vec::new();
    for item in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("resource") => {
                if let Some(text) = item.pointer("/resource/text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some(other) => parts.push(format!("[{other}]")),
            None => {}
        }
    }
    if parts.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        parts.push(structured.to_string());
    }
    CallResult {
        text: parts.join("\n"),
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn reply(id: u64, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    /// A server that answers `initialize` with a session id and accepts
    /// the notification that follows.
    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "initialize" })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200)
                    .insert_header("mcp-session-id", "s1")
                    .set_body_json(reply(
                        id,
                        json!({
                            "protocolVersion": "2025-03-26",
                            "serverInfo": { "name": "linear" },
                            "instructions": "Be brief.",
                            "capabilities": { "tools": {} },
                        }),
                    ))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({ "method": "notifications/initialized" }),
            ))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn opens_a_session_and_reads_what_the_server_says_of_itself() {
        let server = server().await;
        let (_, info) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        assert_eq!(info.name.as_deref(), Some("linear"));
        assert_eq!(info.instructions.as_deref(), Some("Be brief."));
    }

    #[tokio::test]
    async fn sends_the_key_session_and_agreed_version_with_each_request() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .and(header("authorization", "Bearer key"))
            .and(header("mcp-session-id", "s1"))
            .and(header("mcp-protocol-version", "2025-03-26"))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200).set_body_json(reply(id, json!({ "tools": [] })))
            })
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        assert!(client.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reads_a_reply_sent_as_an_event_stream() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                // A progress notification first, as a server may send.
                let body = format!(
                    "event: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
                    json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} }),
                    reply(id, json!({ "tools": [{ "name": "list_issues", "inputSchema": { "type": "object" } }] })),
                );
                ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
            })
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "list_issues");
    }

    #[tokio::test]
    async fn follows_the_cursor_to_the_last_page() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({ "method": "tools/list", "params": { "cursor": "2" } }),
            ))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200)
                    .set_body_json(reply(id, json!({ "tools": [{ "name": "b" }] })))
            })
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200).set_body_json(reply(
                    id,
                    json!({ "tools": [{ "name": "a" }], "nextCursor": "2" }),
                ))
            })
            .with_priority(2)
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        let names: Vec<String> = client
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[tokio::test]
    async fn says_the_key_was_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = McpClient::connect(&server.uri(), "test", TIMEOUT)
            .await
            .err()
            .unwrap();
        assert_eq!(err.to_string(), "401: the key was refused");
    }

    #[tokio::test]
    async fn starts_a_new_session_when_the_server_forgets_the_old_one() {
        let server = server().await;
        // The first tools/list finds the session gone; the second, after
        // a new initialize, is answered.
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(ResponseTemplate::new(404))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200).set_body_json(reply(id, json!({ "tools": [] })))
            })
            .with_priority(2)
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        assert!(client.list_tools().await.unwrap().is_empty());
        let initializes = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.body_json::<Value>().unwrap()["method"] == "initialize")
            .count();
        assert_eq!(initializes, 2);
    }

    #[tokio::test]
    async fn passes_on_a_tool_error_as_an_answer() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call", "params": { "name": "get_issue" } })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200).set_body_json(reply(
                    id,
                    json!({ "content": [{ "type": "text", "text": "No issue NIL-9" }], "isError": true }),
                ))
            })
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        let result = client
            .call_tool("get_issue", json!({ "id": "NIL-9" }))
            .await
            .unwrap();
        assert_eq!(
            result,
            CallResult {
                text: "No issue NIL-9".into(),
                is_error: true
            }
        );
    }

    #[tokio::test]
    async fn surfaces_a_json_rpc_error() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call" })))
            .respond_with(|req: &Request| {
                let id = req.body_json::<Value>().unwrap()["id"].as_u64().unwrap();
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32602, "message": "Unknown tool: nope" },
                }))
            })
            .mount(&server)
            .await;
        let (client, _) = McpClient::connect(&server.uri(), "key", TIMEOUT)
            .await
            .unwrap();
        let err = client.call_tool("nope", json!({})).await.unwrap_err();
        assert!(matches!(err, Error::Rpc { code: -32602, .. }));
    }

    #[test]
    fn marks_destructive_only_where_the_server_says_so() {
        let plain = tool_of(&json!({ "name": "a" })).unwrap();
        assert!(!plain.read_only && !plain.destructive);
        let reads = tool_of(&json!({ "name": "b", "annotations": { "readOnlyHint": true, "destructiveHint": true } })).unwrap();
        assert!(reads.read_only && !reads.destructive);
        let deletes =
            tool_of(&json!({ "name": "c", "annotations": { "destructiveHint": true } })).unwrap();
        assert!(deletes.destructive);
    }

    #[test]
    fn joins_an_events_data_lines() {
        let messages = sse_messages("data: {\"a\":\ndata: 1}\n\n: comment\ndata: {\"b\":2}\n");
        assert_eq!(messages, vec![json!({ "a": 1 }), json!({ "b": 2 })]);
    }

    #[test]
    fn falls_back_to_structured_content_when_there_is_no_text() {
        let result = call_result(&json!({ "content": [], "structuredContent": { "n": 1 } }));
        assert_eq!(result.text, "{\"n\":1}");
    }
}

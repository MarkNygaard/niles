//! The app's chat answered by Claude Code, on the household's own Claude
//! subscription.
//!
//! One `claude -p` per message. Claude Code is an agent, not an API, so
//! Niles cannot hand it tools in a request the way it does Groq;
//! instead Niles serves its tool registry over MCP on a loopback port,
//! and points each run at it with a token that lives for that one turn.
//! The token is also how a tool call knows who is typing: it maps to
//! the speaker the turn was started for, so `remember_about_me` writes
//! to the right person even though the call arrives on another request.
//!
//! Claude Code's own tools are switched off — no shell, no files, no
//! web — and only Niles's are allowed. It runs inside the Niles pod,
//! next to every credential the house has.

use anyhow::{Context, anyhow, bail};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_llm::Message;
use niles_tools::ToolRegistry;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What MCP calls the server, and so the prefix of every tool name
/// Claude Code sees: `mcp__niles__set_device`.
const SERVER_NAME: &str = "niles";

/// Enough for a few tool calls and an answer. A run that needs more is
/// lost, and the timeout ends it anyway.
const MAX_TURNS: u32 = 12;

/// Live turn tokens, and the speaker each was started for.
type Turns = Arc<Mutex<HashMap<String, Option<String>>>>;

pub(crate) struct ClaudeCode {
    mcp_url: String,
    turns: Turns,
}

impl ClaudeCode {
    /// Serve the tools on a loopback port, so only a process in this pod
    /// can reach them — and only with a turn's token.
    pub(crate) async fn start(tools: Arc<ToolRegistry>) -> anyhow::Result<Self> {
        let turns: Turns = Arc::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding the MCP port")?;
        let addr = listener.local_addr()?;
        let app = axum::Router::new()
            .route(
                "/mcp",
                axum::routing::post(serve_mcp).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
            )
            .with_state(McpState {
                tools,
                turns: turns.clone(),
            });
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("[claude-code] the MCP server stopped: {e}");
            }
        });
        Ok(Self {
            mcp_url: format!("http://{addr}/mcp"),
            turns,
        })
    }

    /// Answer one chat message.
    pub(crate) async fn answer(
        &self,
        cfg: &niles_config::ClaudeCodeConfig,
        oauth_token: &str,
        system_prompt: &str,
        history: &[Message],
        text: &str,
        speaker: Option<String>,
    ) -> anyhow::Result<String> {
        let turn = TurnToken::issue(&self.turns, speaker);
        let state = state_dir();
        let work = state.join("work");
        std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;

        let mut command = tokio::process::Command::new("claude");
        command
            .args(arguments(
                cfg,
                system_prompt,
                &transcript(history, text),
                &mcp_config(&self.mcp_url, &turn.0),
            ))
            .env("CLAUDE_CODE_OAUTH_TOKEN", oauth_token)
            .env("CLAUDE_CONFIG_DIR", &state)
            // And its home: some of what it writes goes beside the config
            // directory rather than in it, and the container's user has
            // no home it can write to.
            .env("HOME", &state)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("DISABLE_AUTOUPDATER", "1")
            // An empty directory, so there is no CLAUDE.md or project
            // settings for it to pick up.
            .current_dir(&work)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);

        let output =
            tokio::time::timeout(Duration::from_secs(cfg.timeout_seconds), command.output())
                .await
                .map_err(|_| anyhow!("no answer within {}s", cfg.timeout_seconds))?
                .context("starting `claude` — is Claude Code installed in this image?")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() && stdout.trim().is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "`claude` exited with {}: {}",
                output.status,
                stderr.trim().chars().take(300).collect::<String>()
            );
        }
        parse_result(&stdout)
    }
}

/// Where Claude Code keeps its state. Its own variable when the
/// deployment sets one — a volume, if the temp directory is not
/// writable — and the temp directory otherwise.
fn state_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("niles-claude"))
}

/// A token for one run, withdrawn when the run is over however it ends.
struct TurnToken(String, Turns);

impl TurnToken {
    fn issue(turns: &Turns, speaker: Option<String>) -> Self {
        let token = format!("{:032x}", rand::random::<u128>());
        turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.clone(), speaker);
        Self(token, turns.clone())
    }
}

impl Drop for TurnToken {
    fn drop(&mut self) {
        self.1
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

#[derive(Clone)]
struct McpState {
    tools: Arc<ToolRegistry>,
    turns: Turns,
}

async fn serve_mcp(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(message): Json<Value>,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let speaker = token.and_then(|t| {
        state
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(t)
            .cloned()
    });
    let Some(speaker) = speaker else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match crate::profile::SPEAKER
        .scope(speaker, niles_tools::mcp::handle(&state.tools, &message))
        .await
    {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// The conversation so far and the new message, as one prompt: each run
/// starts fresh, so the history has to travel with it.
fn transcript(history: &[Message], text: &str) -> String {
    let mut out = String::new();
    for message in history {
        match message {
            Message::User { content } => out.push_str(&format!("Person: {content}\n")),
            Message::Assistant {
                content: Some(content),
                ..
            } => out.push_str(&format!("Niles: {content}\n")),
            _ => {}
        }
    }
    if out.is_empty() {
        return text.to_string();
    }
    format!("The conversation so far:\n\n{out}\nThe person now says:\n\n{text}")
}

fn mcp_config(url: &str, token: &str) -> String {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") },
            }
        }
    })
    .to_string()
}

fn arguments(
    cfg: &niles_config::ClaudeCodeConfig,
    system_prompt: &str,
    prompt: &str,
    mcp_config: &str,
) -> Vec<String> {
    [
        "-p",
        prompt,
        "--output-format",
        "json",
        "--model",
        &cfg.model,
        // Replaced, not appended to: Claude Code's own prompt is about
        // writing software in a repository.
        "--system-prompt",
        system_prompt,
        // No built-in tools at all, and only this server's.
        "--tools",
        "",
        "--mcp-config",
        mcp_config,
        "--strict-mcp-config",
        "--allowedTools",
        &format!("mcp__{SERVER_NAME}"),
        // Anything not allowed above is refused rather than waiting on
        // a prompt nobody will see.
        "--permission-mode",
        "dontAsk",
        "--max-turns",
        &MAX_TURNS.to_string(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// The answer out of `--output-format json`.
fn parse_result(stdout: &str) -> anyhow::Result<String> {
    let value: Value = serde_json::from_str(stdout.trim()).with_context(|| {
        format!(
            "`claude` printed something that is not JSON: {}",
            preview(stdout)
        )
    })?;
    let result = value
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let failed = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("subtype")
            .and_then(Value::as_str)
            .is_some_and(|s| s != "success");
    if failed {
        bail!(
            "Claude Code reported an error: {}",
            preview(if result.is_empty() { stdout } else { result })
        );
    }
    if result.is_empty() {
        bail!("Claude Code answered with nothing");
    }
    Ok(result.to_string())
}

fn preview(text: &str) -> String {
    text.trim().chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> niles_config::ClaudeCodeConfig {
        toml::from_str("").expect("every field has a default")
    }

    #[test]
    fn a_first_message_is_sent_as_it_is() {
        assert_eq!(transcript(&[], "is anyone home?"), "is anyone home?");
    }

    #[test]
    fn a_follow_up_carries_the_conversation() {
        let history = vec![
            Message::User {
                content: "add milk".into(),
            },
            Message::Assistant {
                content: Some("Added milk, Sir.".into()),
                tool_calls: None,
            },
        ];
        let prompt = transcript(&history, "and eggs");
        assert!(
            prompt.contains("Person: add milk\nNiles: Added milk, Sir.\n"),
            "{prompt}"
        );
        assert!(prompt.ends_with("and eggs"), "{prompt}");
    }

    #[test]
    fn claude_code_gets_no_tools_of_its_own() {
        let args = arguments(&cfg(), "You are Niles.", "hi", "{}");
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).expect(flag);
            args[i + 1].as_str()
        };
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--allowedTools"), "mcp__niles");
        assert_eq!(after("--permission-mode"), "dontAsk");
        assert_eq!(after("--system-prompt"), "You are Niles.");
        assert_eq!(after("--model"), "sonnet");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn the_mcp_config_carries_the_turn_token() {
        let config: Value =
            serde_json::from_str(&mcp_config("http://127.0.0.1:1/mcp", "abc")).unwrap();
        let server = &config["mcpServers"]["niles"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["headers"]["Authorization"], "Bearer abc");
    }

    #[test]
    fn reads_the_answer() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"Only you, Sir.","session_id":"x"}"#;
        assert_eq!(parse_result(out).unwrap(), "Only you, Sir.");
    }

    #[test]
    fn an_error_run_is_an_error() {
        let out = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#;
        assert!(parse_result(out).is_err());
    }

    #[test]
    fn something_that_is_not_json_is_an_error() {
        assert!(parse_result("Invalid API key · Please run /login").is_err());
    }

    #[test]
    fn a_token_is_withdrawn_when_the_turn_ends() {
        let turns: Turns = Arc::default();
        let token = TurnToken::issue(&turns, Some("mark".into()));
        let key = token.0.clone();
        assert_eq!(
            turns.lock().unwrap().get(&key),
            Some(&Some("mark".to_string()))
        );
        drop(token);
        assert!(turns.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_mcp_port_wants_a_live_token() {
        let claude = ClaudeCode::start(Arc::new(ToolRegistry::new()))
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

        let refused = client
            .post(&claude.mcp_url)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);

        let token = TurnToken::issue(&claude.turns, None);
        let answered = client
            .post(&claude.mcp_url)
            .bearer_auth(&token.0)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(answered.status(), reqwest::StatusCode::OK);
        let body: Value = answered.json().await.unwrap();
        assert_eq!(body["result"], json!({}));
    }

    /// Real Claude Code against the real MCP server and the real grocery
    /// tools. Ignored: it needs `claude` on the PATH and a token from
    /// `claude setup-token`, and spends a few tokens:
    ///
    /// `CLAUDE_CODE_OAUTH_TOKEN=… cargo test -p niles-bin -- --ignored claude_code`
    ///
    /// Not with `CLAUDE_CONFIG_DIR` pointed at a real `~/.claude`: the run
    /// writes its own state there.
    #[tokio::test]
    #[ignore]
    async fn a_real_claude_code_adds_to_the_list() {
        let store = Arc::new(niles_groceries::GroceryStore::new());
        let mut registry = ToolRegistry::new();
        niles_tools::register_grocery_tools(&mut registry, store.clone(), Some("DK".into()));
        let claude = ClaudeCode::start(Arc::new(registry)).await.unwrap();
        let mut cfg = cfg();
        cfg.model = "haiku".into();
        let token = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").unwrap_or_default();

        let reply = claude
            .answer(
                &cfg,
                &token,
                "You are Niles, a household assistant. Use your tools to act.",
                &[],
                "Add milk to the shopping list.",
                None,
            )
            .await
            .expect("an answer");

        let items = store.list();
        assert_eq!(items.len(), 1, "reply was: {reply}");
        // In its words when it translated ("Mælk", said "milk"), or as
        // the name when it did not ("Milk").
        let asked = items[0]
            .said
            .clone()
            .unwrap_or_else(|| items[0].name.clone());
        assert_eq!(asked.to_lowercase(), "milk", "{:?}", items[0]);
    }
}

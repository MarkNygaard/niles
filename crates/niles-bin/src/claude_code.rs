//! Claude Code as the assistant, on the household's own Claude
//! subscription: one `claude -p` per run, pointed at Niles's tools over
//! MCP (see [`crate::assistant`]).
//!
//! Claude Code's own tools are switched off — no shell, no web, no
//! files — with two exceptions. When the person attached files to the
//! message, `Read` is allowed, for that run only, and `--restricted`
//! confines it to the directory the run works in, which holds nothing
//! but this turn's files. And when the house has no SearXNG, its own
//! `WebSearch` is allowed in place of Niles's `web_search`, in a run
//! that would have had that. `--permission-mode dontAsk` means anything
//! else is refused rather than waiting on a prompt nobody will see. It
//! runs inside the Niles pod, next to every credential the house has.
//!
//! `WebFetch` stays off whatever else is on: it fetches any address it
//! is given from inside the pod, which can reach the cluster's own
//! services. A page is read with Niles's `read_page`, which goes only to
//! the public internet. `WebSearch` searches on Anthropic's side and
//! brings back results, so it reaches nothing here; `--restricted` takes
//! away only the tools that run code, and `WebFetch`, and leaves it be
//! when `--tools` names it.

use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

/// What MCP calls the server, and so the prefix of every tool name
/// Claude Code sees: `mcp__niles__set_device`.
const SERVER_NAME: &str = "niles";

/// Enough for a few tool calls and an answer. A run that needs more is
/// lost, and the timeout ends it anyway.
const MAX_TURNS: u32 = 12;

/// The background jobs — the review and the tidy-up — get the strongest
/// model whatever the chat uses: nobody waits for them, and their
/// judgement is what goes into every later prompt.
const BACKGROUND_MODEL: &str = "opus";
const BACKGROUND_TURNS: u32 = 80;
const BACKGROUND_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// How one run of `claude` is sized.
pub(crate) struct Run<'a> {
    model: &'a str,
    max_turns: u32,
    timeout: Duration,
}

impl<'a> Run<'a> {
    pub(crate) fn chat(cfg: &'a niles_config::ClaudeCodeConfig) -> Self {
        Self {
            model: &cfg.model,
            max_turns: MAX_TURNS,
            timeout: Duration::from_secs(cfg.timeout_seconds),
        }
    }

    pub(crate) fn background() -> Self {
        Self {
            model: BACKGROUND_MODEL,
            max_turns: BACKGROUND_TURNS,
            timeout: BACKGROUND_TIMEOUT,
        }
    }
}

/// One run. `web_search` gives it Claude Code's own `WebSearch`: for a
/// run that searches, when the house has no SearXNG.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    run: Run<'_>,
    oauth_token: &str,
    mcp_url: &str,
    turn_token: &str,
    system_prompt: &str,
    prompt: &str,
    attachments: &[crate::assistant::Attached],
    web_search: bool,
) -> anyhow::Result<String> {
    let state = state_dir()?;
    let work = state.join("work");
    let staged = if attachments.is_empty() {
        None
    } else {
        Some(crate::assistant::stage(&work, attachments)?)
    };
    let cwd = staged
        .as_ref()
        .map_or_else(|| work.clone(), |s| s.dir.clone());

    let mut command = tokio::process::Command::new("claude");
    command
        .args(arguments(
            &run,
            system_prompt,
            prompt,
            &mcp_config(mcp_url, turn_token),
            !attachments.is_empty(),
            web_search,
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
        // settings for it to pick up — or, with files attached, the
        // staging directory holding exactly those files.
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);

    let output = crate::assistant::within(run.timeout, async {
        command
            .output()
            .await
            .context("starting `claude` — is Claude Code installed in this image?")
    })
    .await?;
    // `staged` drops here, after the run has ended, taking the
    // directory and its files with it.

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

/// Where Claude Code keeps its state: the first of these it can write.
///
/// Its own variable when the deployment sets one; then the data volume,
/// which a pod with a read-only root still has; then the temp directory,
/// which is where a development machine has room.
fn state_dir() -> anyhow::Result<PathBuf> {
    // The volume only where there is one: on a development machine
    // "/data" would be a new folder at the root of the drive.
    let volume = std::path::Path::new("/data")
        .is_dir()
        .then(|| PathBuf::from("/data/claude"));
    let candidates: Vec<PathBuf> = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .into_iter()
        .chain(volume)
        .chain([std::env::temp_dir().join("niles-claude")])
        .collect();
    let mut tried = Vec::new();
    for dir in candidates {
        match std::fs::create_dir_all(dir.join("work")) {
            Ok(()) => return Ok(dir),
            Err(e) => tried.push(format!("{} ({e})", dir.display())),
        }
    }
    bail!("nowhere to keep its state: tried {}", tried.join(", "))
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
    run: &Run<'_>,
    system_prompt: &str,
    prompt: &str,
    mcp_config: &str,
    with_files: bool,
    web_search: bool,
) -> Vec<String> {
    let own: Vec<&str> = [(with_files, "Read"), (web_search, "WebSearch")]
        .into_iter()
        .filter_map(|(on, tool)| on.then_some(tool))
        .collect();
    let tools = own.join(",");
    let allowed = std::iter::once(format!("mcp__{SERVER_NAME}"))
        .chain(own.iter().map(|tool| tool.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let mut args: Vec<String> = [
        "-p",
        prompt,
        "--output-format",
        "json",
        "--model",
        run.model,
        // Replaced, not appended to: Claude Code's own prompt is about
        // writing software in a repository.
        "--system-prompt",
        system_prompt,
        // No built-in tools at all, and only this server's — or, with
        // files attached, `Read` as well, confined by `--restricted`, and
        // without a SearXNG, `WebSearch`. Never `WebFetch`.
        "--tools",
        &tools,
        "--mcp-config",
        mcp_config,
        "--strict-mcp-config",
        "--allowedTools",
        &allowed,
        // Anything not allowed above is refused rather than waiting on
        // a prompt nobody will see.
        "--permission-mode",
        "dontAsk",
        "--max-turns",
        &run.max_turns.to_string(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if with_files {
        args.push("--restricted".into());
    }
    args
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
    use std::sync::Arc;

    fn cfg() -> niles_config::ClaudeCodeConfig {
        toml::from_str("").expect("every field has a default")
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).expect(flag);
        args[i + 1].as_str()
    }

    #[test]
    fn without_searxng_claude_searches_with_its_own_and_fetches_nothing() {
        let cfg = cfg();
        let args = arguments(&Run::chat(&cfg), "s", "p", "{}", false, true);
        assert_eq!(after(&args, "--tools"), "WebSearch");
        assert_eq!(after(&args, "--allowedTools"), "mcp__niles,WebSearch");
        assert!(!args.iter().any(|a| a.contains("WebFetch")), "{args:?}");
        assert!(!args.contains(&"--restricted".to_string()));
        assert_eq!(after(&args, "--permission-mode"), "dontAsk");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn with_searxng_claude_has_no_search_of_its_own() {
        let cfg = cfg();
        for files in [false, true] {
            let args = arguments(&Run::chat(&cfg), "s", "p", "{}", files, false);
            assert!(!after(&args, "--tools").contains("WebSearch"), "{args:?}");
            assert!(
                !after(&args, "--allowedTools").contains("WebSearch"),
                "{args:?}"
            );
        }
    }

    #[test]
    fn files_and_its_own_search_together_are_still_restricted() {
        let cfg = cfg();
        let args = arguments(&Run::chat(&cfg), "s", "p", "{}", true, true);
        assert_eq!(after(&args, "--tools"), "Read,WebSearch");
        assert_eq!(after(&args, "--allowedTools"), "mcp__niles,Read,WebSearch");
        assert!(args.contains(&"--restricted".to_string()));
        assert!(!args.iter().any(|a| a.contains("WebFetch")), "{args:?}");
    }

    #[test]
    fn claude_code_gets_no_tools_of_its_own() {
        let cfg = cfg();
        let args = arguments(&Run::chat(&cfg), "You are Niles.", "hi", "{}", false, false);
        let after = |flag: &str| after(&args, flag);
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--allowedTools"), "mcp__niles");
        assert_eq!(after("--permission-mode"), "dontAsk");
        assert_eq!(after("--system-prompt"), "You are Niles.");
        assert_eq!(after("--model"), "sonnet");
        assert_eq!(after("--max-turns"), "12");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert!(!args.contains(&"--restricted".to_string()));
    }

    #[test]
    fn files_bring_read_confined_to_the_run_directory() {
        let cfg = cfg();
        let args = arguments(&Run::chat(&cfg), "s", "p", "{}", true, false);
        let after = |flag: &str| after(&args, flag);
        assert_eq!(after("--tools"), "Read");
        assert_eq!(after("--allowedTools"), "mcp__niles,Read");
        assert!(args.contains(&"--restricted".to_string()));
        // Everything else is as it was: the rest of the run is
        // unchanged by having files.
        assert_eq!(after("--permission-mode"), "dontAsk");
        assert_eq!(after("--system-prompt"), "s");
        assert_eq!(after("--model"), "sonnet");
        assert_eq!(after("--max-turns"), "12");
    }

    #[test]
    fn the_background_gets_the_strongest_model() {
        let args = arguments(&Run::background(), "s", "p", "{}", false, false);
        let i = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[i + 1], "opus");
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
        let mut registry = niles_tools::ToolRegistry::new();
        niles_tools::register_grocery_tools(&mut registry, store.clone(), Some("DK".into()));
        let assistant = crate::assistant::Assistant::start(
            Arc::new(registry),
            niles_tools::ToolRegistry::new(),
            None,
            niles_tools::Search::none(),
        )
        .await
        .unwrap();
        let mut cfg = cfg();
        cfg.model = "haiku".into();
        let chosen = crate::assistant::Chosen::ClaudeCode {
            cfg,
            token: std::env::var("CLAUDE_CODE_OAUTH_TOKEN").unwrap_or_default(),
        };

        let reply = assistant
            .answer(
                &chosen,
                "You are Niles, a household assistant. Use your tools to act.",
                &[],
                "Add milk to the shopping list.",
                None,
                &[],
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

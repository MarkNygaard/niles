//! ChatGPT as the assistant, on the household's own ChatGPT
//! subscription: one run of the pi coding agent per turn, signed in with
//! the tokens from the app's ChatGPT sign-in.
//!
//! pi speaks no MCP, so Niles's tools reach it through a pi extension
//! that does: `deploy/pi/niles.ts`, which lists the tools on Niles's
//! loopback MCP server and calls them there (see [`crate::assistant`]).
//! The run's token travels in its environment, so the grant is the same
//! one Claude Code gets — the same speaker, the same background-only
//! tools.
//!
//! pi comes with a shell and a file editor. They are switched off, and
//! its environment is emptied before it starts: it runs inside the
//! Niles pod, beside the database URL and the secrets key.

use anyhow::{Context, anyhow, bail};
use niles_config::ChatGptSignIn;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// pi's name for the ChatGPT-subscription provider, and the key its
/// `auth.json` keeps that sign-in under.
const PROVIDER: &str = "openai-codex";

/// The review and the tidy-up get the most capable model, as Claude's
/// do, and time to think: nobody is waiting on them.
const BACKGROUND_MODEL: &str = "gpt-6-astra";
const BACKGROUND_THINKING: &str = "medium";
const BACKGROUND_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// pi's own tools, every one of which reaches the pod's files or shell.
/// Named as well as switched off: `--no-builtin-tools` only leaves them
/// inactive, where an extension could turn them back on, and naming
/// them keeps them out of the run altogether.
const BUILTIN_TOOLS: &[&str] = &[
    "read",
    "bash",
    "powershell",
    "edit",
    "write",
    "grep",
    "find",
    "ls",
];

/// Where the image puts the extension, alone in its directory.
const EXTENSION: &str = "/usr/local/lib/niles/pi/niles.ts";

/// pi puts whole tool results in its event stream, so a line can be
/// large. One beyond this is dropped rather than read: it is a tool
/// result the model has already seen.
const LINE_LIMIT: usize = 16 * 1024 * 1024;

/// Renewed this long before it runs out, so a run never starts on a
/// token that lapses half way through.
const RENEW_WITHIN: i64 = 5 * 60;

/// Held while renewing. OpenAI rotates the refresh token on every use,
/// so two renewals at once would each spend it and the second would sign
/// the house out.
static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How one run of `pi` is sized. pi has no turn limit; the clock is the
/// only one.
pub(crate) struct Run<'a> {
    model: &'a str,
    thinking: Option<&'static str>,
    timeout: Duration,
}

impl<'a> Run<'a> {
    /// No thinking level for the chat: pi then uses the model's own,
    /// and a person is waiting.
    pub(crate) fn chat(cfg: &'a niles_config::ChatGptConfig) -> Self {
        Self {
            model: &cfg.model,
            thinking: None,
            timeout: Duration::from_secs(cfg.timeout_seconds),
        }
    }

    pub(crate) fn background() -> Self {
        Self {
            model: BACKGROUND_MODEL,
            thinking: Some(BACKGROUND_THINKING),
            timeout: BACKGROUND_TIMEOUT,
        }
    }
}

pub(crate) async fn run(
    run: Run<'_>,
    secrets: Option<&niles_db::PostgresSecrets>,
    mcp_url: &str,
    turn_token: &str,
    system_prompt: &str,
    prompt: &str,
    attachments: &[crate::assistant::Attached],
) -> anyhow::Result<String> {
    let sign_in = fresh_sign_in(secrets).await?;
    let home = pi_home()?;
    write_auth(&home, &sign_in)?;
    let extension = extension()?;
    let staged = if attachments.is_empty() {
        None
    } else {
        Some(crate::assistant::stage(&home.join("work"), attachments)?)
    };
    let cwd = staged
        .as_ref()
        .map_or_else(|| home.join("work"), |s| s.dir.clone());

    let mut command = tokio::process::Command::new("pi");
    command
        .args(arguments(
            &run,
            &extension,
            system_prompt,
            &staged.as_ref().map_or_else(Vec::new, |s| {
                attachments
                    .iter()
                    .map(|a| s.dir.join(&a.file))
                    .collect::<Vec<_>>()
            }),
        ))
        .env_clear()
        .envs(environment(&home, mcp_url, turn_token, |key| {
            std::env::var(key).ok()
        }))
        // An empty directory, so there is no AGENTS.md or project
        // settings for it to pick up — or, with files attached, the
        // staging directory holding exactly those files.
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let output = crate::assistant::within(run.timeout, async {
        let mut child = command
            .spawn()
            .context("starting `pi` — is it installed in this image?")?;
        // The message goes in on stdin, never as an argument: pi reads an
        // argument starting with "@" as a file to attach, and a person
        // typing "@/proc/1/environ" would have it read Niles's.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.ok();
        }
        child.wait_with_output().await.context("waiting for `pi`")
    })
    .await?;
    // `staged` drops here, after the run has ended, taking the
    // directory and its files with it.

    parse_events(&output.stdout, &output.stderr, output.status.code())
}

/// The stored sign-in, renewed first when it is about to run out.
///
/// Niles alone renews it. pi is never given the refresh token: if it
/// renewed, the token Niles holds would be spent, and the next renewal
/// would fail with `refresh_token_reused` and sign the house out.
async fn fresh_sign_in(
    secrets: Option<&niles_db::PostgresSecrets>,
) -> anyhow::Result<ChatGptSignIn> {
    let stored = ChatGptSignIn::stored().ok_or_else(|| anyhow!("ChatGPT is not signed in"))?;
    if !expiring(&stored, now()) {
        return Ok(stored);
    }
    let _renewing = RENEWING.lock().await;
    let store = secrets.ok_or_else(|| {
        anyhow!("the ChatGPT sign-in has run out, and without a database there is nowhere to keep a renewed one")
    })?;
    // Read again under the lock: a run that waited here may find the one
    // before it has already renewed.
    niles_config::secrets::load(store.load_all().await.context("reading the sign-in")?);
    let stored = ChatGptSignIn::stored().ok_or_else(|| anyhow!("ChatGPT is not signed in"))?;
    if !expiring(&stored, now()) {
        return Ok(stored);
    }
    let tokens = niles_llm::chatgpt::refresh(
        &reqwest::Client::new(),
        niles_llm::chatgpt::TOKEN_URL,
        &stored.refresh_token,
    )
    .await
    .context("renewing the ChatGPT sign-in — signing in again from its card will fix it")?;
    let renewed = ChatGptSignIn {
        account_id: niles_llm::chatgpt::account_id(&tokens.access_token)
            .unwrap_or(stored.account_id),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token.unwrap_or(stored.refresh_token),
        expires_at: now() + i64::try_from(tokens.expires_in).unwrap_or(3600),
    };
    for (key, value) in renewed.entries() {
        store
            .set(key, &value)
            .await
            .context("keeping the renewed ChatGPT sign-in")?;
    }
    niles_config::secrets::load(store.load_all().await.context("reading the sign-in")?);
    tracing::info!("[chatgpt] renewed the sign-in");
    Ok(renewed)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn expiring(sign_in: &ChatGptSignIn, now: i64) -> bool {
    sign_in.expires_at - now <= RENEW_WITHIN
}

/// pi's home: the data volume when there is one, else the temp
/// directory. pi keeps its sign-in and settings under `~/.pi`, and the
/// container's user has no home it can write to.
fn pi_home() -> anyhow::Result<PathBuf> {
    // The volume only where there is one: on a development machine
    // "/data" would be a new folder at the root of the drive.
    let volume = Path::new("/data")
        .is_dir()
        .then(|| PathBuf::from("/data/pi"));
    let mut tried = Vec::new();
    for dir in volume
        .into_iter()
        .chain([std::env::temp_dir().join("niles-pi")])
    {
        match std::fs::create_dir_all(dir.join("work")) {
            Ok(()) => return Ok(dir),
            Err(e) => tried.push(format!("{} ({e})", dir.display())),
        }
    }
    bail!(
        "nowhere for pi to keep its state: tried {}",
        tried.join(", ")
    )
}

/// The sign-in as pi reads it from `~/.pi/agent/auth.json`.
fn auth_json(sign_in: &ChatGptSignIn) -> Value {
    json!({
        PROVIDER: {
            "type": "oauth",
            "access": sign_in.access_token,
            "accountId": sign_in.account_id,
            // pi counts in milliseconds.
            "expires": sign_in.expires_at.saturating_mul(1000),
        }
    })
}

/// Written before every run, readable by Niles's user alone: it is a
/// live token. Replaced whole rather than rewritten, so a run starting
/// beside another never reads half of one.
fn write_auth(home: &Path, sign_in: &ChatGptSignIn) -> anyhow::Result<()> {
    let dir = home.join(".pi").join("agent");
    std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    let path = dir.join("auth.json");
    let partial = dir.join(format!("auth.json.{:x}", rand::random::<u64>()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&partial).and_then(|mut file| {
        std::io::Write::write_all(&mut file, auth_json(sign_in).to_string().as_bytes())
    });
    written
        .and_then(|()| std::fs::rename(&partial, &path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&partial);
        })
        .with_context(|| format!("writing {}", path.display()))
}

/// The extension: the image's, or the repository's on a development
/// machine.
fn extension() -> anyhow::Result<PathBuf> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/pi/niles.ts");
    [PathBuf::from(EXTENSION), repository]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("pi's Niles extension is missing from {EXTENSION}"))
}

fn arguments(
    run: &Run<'_>,
    extension: &Path,
    system_prompt: &str,
    paths: &[PathBuf],
) -> Vec<String> {
    let mut args: Vec<String> = [
        // Nothing found lying around: no extensions, skills, prompt
        // templates or AGENTS.md but what is named here, and no session
        // file left behind per message.
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-context-files",
        "--no-session",
        "--no-builtin-tools",
        "--exclude-tools",
        &BUILTIN_TOOLS.join(","),
        "--extension",
        &extension.to_string_lossy(),
        "--model",
        &format!("{PROVIDER}/{}", run.model),
        "--mode",
        "json",
        // Replaced, not appended to: pi's own prompt is about writing
        // software, and about where pi's documentation lives.
        "--system-prompt",
        system_prompt,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if let Some(level) = run.thinking {
        args.extend(["--thinking".to_string(), level.to_string()]);
    }
    // Last, with nothing after it: the message comes on stdin. A
    // staged attachment is the one exception — pi reads an argument
    // starting with `@` as a file to attach, which is exactly how a
    // file reaches it. That is also exactly why the person's words
    // still never become an argument. `read` stays excluded above:
    // an `@` argument is the CLI attaching a file, not pi reading
    // one.
    args.push("-p".into());
    for path in paths {
        args.push(format!("@{}", path.display()));
    }
    args
}

/// Everything pi's process may see of the world. An allowlist, so a
/// secret added to the deployment tomorrow is left out because nobody
/// listed it, rather than let in because nobody remembered to block it.
pub(crate) fn environment(
    home: &Path,
    mcp_url: &str,
    turn_token: &str,
    parent: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = ["PATH", "LANG", "TZ"]
        .into_iter()
        .filter_map(|key| parent(key).map(|value| (key.to_string(), value)))
        .collect();
    env.extend([
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
        ("NILES_MCP_URL".to_string(), mcp_url.to_string()),
        ("NILES_MCP_TOKEN".to_string(), turn_token.to_string()),
    ]);
    env
}

/// The answer out of pi's `--mode json` events.
///
/// The last assistant message with text in it is the answer. pi exits 0
/// when the provider turns a request down — a lapsed subscription, a
/// model the plan does not include — and the only trace in the events
/// is a message with no text and `stopReason: "error"`. So no text is a
/// failure however pi exited, and stderr is where the reason is, when
/// there is one: its first lines, since a Node crash ends with a version
/// banner that would hide the cause.
fn parse_events(stdout: &[u8], stderr: &[u8], code: Option<i32>) -> anyhow::Result<String> {
    let mut answer = String::new();
    let mut stop_reason = None;
    let mut error_message = None;
    for line in stdout.split(|b| *b == b'\n') {
        if line.len() > LINE_LIMIT {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<Value>(line.trim_ascii()) else {
            continue;
        };
        let message = &event["message"];
        if event["type"] != "message_end" || message["role"] != "assistant" {
            continue;
        }
        if let Some(reason) = message["stopReason"].as_str() {
            stop_reason = Some(reason.to_string());
        }
        if let Some(error) = message["errorMessage"].as_str().filter(|e| !e.is_empty()) {
            error_message = Some(error.to_string());
        }
        let text: String = message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect();
        if !text.trim().is_empty() {
            answer = text;
        }
    }
    if !answer.trim().is_empty() {
        return Ok(answer.trim().to_string());
    }
    let stderr = String::from_utf8_lossy(stderr);
    let first: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(6)
        .collect();
    let detail = if !first.is_empty() {
        first.join(" | ")
    } else if let Some(error) = error_message {
        error
    } else {
        format!(
            "no answer (stopReason {}); usually the sign-in or the model was refused",
            stop_reason.as_deref().unwrap_or("unknown")
        )
    };
    match code {
        Some(code) => bail!("pi exited {code}: {detail}"),
        None => bail!("pi was stopped: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg() -> niles_config::ChatGptConfig {
        toml::from_str("").expect("every field has a default")
    }

    fn sign_in() -> ChatGptSignIn {
        ChatGptSignIn {
            access_token: "at-live".into(),
            refresh_token: "rt-secret".into(),
            account_id: "acct-1".into(),
            expires_at: 1_900_000_000,
        }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).expect(flag);
        args[i + 1].as_str()
    }

    #[test]
    fn pi_gets_none_of_its_own_tools_and_only_the_niles_extension() {
        let cfg = cfg();
        let args = arguments(
            &Run::chat(&cfg),
            Path::new("/usr/local/lib/niles/pi/niles.ts"),
            "You are Niles.",
            &[],
        );
        assert!(args.contains(&"--no-builtin-tools".to_string()), "{args:?}");
        let excluded: Vec<&str> = after(&args, "--exclude-tools").split(',').collect();
        for tool in ["read", "bash", "edit", "write", "grep", "find", "ls"] {
            assert!(excluded.contains(&tool), "{tool} left in: {excluded:?}");
        }
        assert!(args.contains(&"--no-extensions".to_string()), "{args:?}");
        assert_eq!(
            after(&args, "--extension"),
            "/usr/local/lib/niles/pi/niles.ts"
        );
        assert_eq!(
            args.iter()
                .filter(|a| *a == "--extension" || *a == "-e")
                .count(),
            1
        );
        assert!(args.contains(&"--no-context-files".to_string()), "{args:?}");
        assert!(args.contains(&"--no-skills".to_string()), "{args:?}");
    }

    #[test]
    fn the_chat_runs_the_chosen_model_without_extra_thinking() {
        let cfg = cfg();
        let args = arguments(&Run::chat(&cfg), Path::new("x.ts"), "You are Niles.", &[]);
        assert_eq!(after(&args, "--model"), "openai-codex/gpt-5.6-terra");
        assert_eq!(after(&args, "--mode"), "json");
        assert_eq!(after(&args, "--system-prompt"), "You are Niles.");
        assert!(!args.contains(&"--thinking".to_string()), "{args:?}");
    }

    #[test]
    fn the_background_gets_the_most_capable_model_and_time_to_think() {
        let args = arguments(&Run::background(), Path::new("x.ts"), "s", &[]);
        assert_eq!(after(&args, "--model"), "openai-codex/gpt-6-astra");
        assert_eq!(after(&args, "--thinking"), "medium");
    }

    #[test]
    fn the_message_is_never_an_argument() {
        // pi takes an argument starting with "@" as a file to attach.
        let args = arguments(&Run::background(), Path::new("x.ts"), "s", &[]);
        assert_eq!(args.last().map(String::as_str), Some("-p"));
        assert!(!args.iter().any(|a| a.starts_with('@')), "{args:?}");
    }

    #[test]
    fn attachments_are_the_only_arguments_and_read_stays_out() {
        // With files, every argument after `-p` is one of the staged
        // paths, and none holds the message.
        let paths: Vec<PathBuf> = ["/tmp/stage/a1.jpg", "/tmp/stage/a2.pdf"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let args = arguments(&Run::background(), Path::new("x.ts"), "s", &paths);
        let p = args.iter().position(|a| a == "-p").unwrap();
        let rest = &args[p + 1..];
        assert_eq!(rest.len(), 2, "{args:?}");
        for (arg, path) in rest.iter().zip(paths.iter()) {
            assert_eq!(arg, &format!("@{}", path.display()));
        }
        assert!(!rest.iter().any(|a| a == "s"), "{args:?}");
        // `read` is still excluded: an `@` argument attaches a file,
        // it is not pi reading one.
        let excluded: Vec<&str> = after(&args, "--exclude-tools").split(',').collect();
        assert!(excluded.contains(&"read"), "{excluded:?}");
    }

    #[test]
    fn pi_sees_the_allowlist_and_nothing_else() {
        let parent: HashMap<&str, &str> = HashMap::from([
            ("PATH", "/usr/local/bin:/usr/bin"),
            ("LANG", "C.UTF-8"),
            ("DATABASE_URL", "postgres://niles:secret@db/niles"),
            ("NILES_SECRETS_KEY", "k3y"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant"),
        ]);
        let env: HashMap<String, String> = environment(
            Path::new("/data/pi"),
            "http://127.0.0.1:4242/mcp",
            "turn-token",
            |key| parent.get(key).map(|v| v.to_string()),
        )
        .into_iter()
        .collect();
        let mut keys: Vec<&str> = env.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["HOME", "LANG", "NILES_MCP_TOKEN", "NILES_MCP_URL", "PATH"]
        );
        assert_eq!(env["HOME"], "/data/pi");
        assert_eq!(env["NILES_MCP_URL"], "http://127.0.0.1:4242/mcp");
        assert_eq!(env["NILES_MCP_TOKEN"], "turn-token");
        assert_eq!(env["PATH"], "/usr/local/bin:/usr/bin");
    }

    #[test]
    fn auth_json_holds_the_access_token_and_never_the_refresh_token() {
        let auth = auth_json(&sign_in());
        let entry = &auth["openai-codex"];
        assert_eq!(entry["type"], "oauth");
        assert_eq!(entry["access"], "at-live");
        assert_eq!(entry["accountId"], "acct-1");
        assert_eq!(entry["expires"], 1_900_000_000_000_i64);
        assert!(entry.get("refresh").is_none(), "{auth}");
        assert!(!auth.to_string().contains("rt-secret"), "{auth}");
    }

    #[test]
    fn auth_json_is_written_where_pi_looks_and_for_niles_alone() {
        let home = tempfile::tempdir().unwrap();
        write_auth(home.path(), &sign_in()).unwrap();
        let path = home.path().join(".pi/agent/auth.json");
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written, auth_json(&sign_in()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Written again over the last one, as every run does.
        write_auth(home.path(), &sign_in()).unwrap();
        let left: Vec<_> = std::fs::read_dir(home.path().join(".pi/agent"))
            .unwrap()
            .collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn a_sign_in_is_renewed_five_minutes_before_it_runs_out() {
        let mut s = sign_in();
        s.expires_at = 10_000;
        assert!(!expiring(&s, 10_000 - 301));
        assert!(expiring(&s, 10_000 - 300));
        assert!(expiring(&s, 20_000));
    }

    #[test]
    fn reads_the_answer_from_the_last_message_with_text() {
        let events = [
            json!({"type": "agent_start"}),
            json!({"type": "tool_execution_start", "toolName": "add_grocery", "toolCallId": "a", "args": {}}),
            json!({"type": "message_update", "message": {"role": "assistant", "content": [{"type": "text", "text": "Add"}]}}),
            json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "toolUse",
                "content": [{"type": "text", "text": "Let me add that."}, {"type": "toolCall", "name": "add_grocery"}]}}),
            json!({"type": "tool_execution_end", "toolCallId": "a"}),
            json!({"type": "message_end", "message": {"role": "toolResult", "content": [{"type": "text", "text": "{\"ok\":true}"}]}}),
            json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "stop",
                "content": [{"type": "thinking", "thinking": "done"}, {"type": "text", "text": "Added milk, Sir."}]}}),
            json!({"type": "agent_end"}),
        ];
        let stdout: String = events.iter().map(|e| format!("{e}\n")).collect();
        let stdout = format!("not json at all\n{stdout}");
        assert_eq!(
            parse_events(stdout.as_bytes(), b"", Some(0)).unwrap(),
            "Added milk, Sir."
        );
    }

    #[test]
    fn a_refused_request_is_an_error_though_pi_exits_0() {
        // Measured in home-ops-agent against a spent credential: exit 0,
        // nothing on stderr, and an assistant message with no text.
        let stdout = format!(
            "{}\n{}\n",
            json!({"type": "agent_start"}),
            json!({"type": "message_end", "message": {"role": "assistant", "content": [],
                "stopReason": "error", "usage": {"input": 0, "output": 0}}}),
        );
        let err = parse_events(stdout.as_bytes(), b"", Some(0)).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("pi exited 0"), "{err}");
        assert!(err.contains("stopReason error"), "{err}");
    }

    #[test]
    fn the_providers_own_words_are_used_when_pi_has_them() {
        let stdout = format!(
            "{}\n",
            json!({"type": "message_end", "message": {"role": "assistant", "content": [],
                "stopReason": "error", "errorMessage": "Your plan does not include this model"}}),
        );
        let err = parse_events(stdout.as_bytes(), b"", Some(0)).unwrap_err();
        assert!(err.to_string().contains("does not include"), "{err}");
    }

    #[test]
    fn stderr_is_reported_from_its_first_lines() {
        let stderr = "Error: ENOENT: no such file or directory, mkdir '/home/niles/.pi'\n    at mkdirSync\n\n    at a\n    at b\n    at c\n    at d\nNode.js v22.23.2\n";
        let err = parse_events(b"", stderr.as_bytes(), Some(1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("pi exited 1"), "{err}");
        assert!(err.contains("ENOENT"), "{err}");
        assert!(!err.contains("Node.js v22"), "{err}");
    }

    #[test]
    fn an_answer_stands_beside_noise_on_stderr() {
        let stdout = format!(
            "{}\n",
            json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "stop",
                "content": [{"type": "text", "text": "Done."}]}}),
        );
        assert_eq!(
            parse_events(stdout.as_bytes(), b"[niles] a warning", Some(0)).unwrap(),
            "Done."
        );
    }

    /// Real pi against the real MCP server and the real grocery tools,
    /// on a real ChatGPT sign-in. Ignored: it needs `pi` and `node` on
    /// the PATH, and the four `integrations.chatgpt.*` values from a
    /// sign-in in these variables — `NILES_TEST_CHATGPT_ACCESS`,
    /// `_REFRESH`, `_ACCOUNT`, `_EXPIRES` (epoch seconds, far enough
    /// out that this does not try to renew):
    ///
    /// `cargo test -p niles-bin -- --ignored a_real_chatgpt`
    #[tokio::test]
    #[ignore]
    async fn a_real_chatgpt_adds_to_the_list() {
        use std::sync::Arc;
        let var = |name: &str| std::env::var(format!("NILES_TEST_CHATGPT_{name}")).unwrap();
        let signed_in = ChatGptSignIn {
            access_token: var("ACCESS"),
            refresh_token: var("REFRESH"),
            account_id: var("ACCOUNT"),
            expires_at: var("EXPIRES").parse().unwrap(),
        };
        niles_config::secrets::load(
            signed_in
                .entries()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
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
        let chosen = crate::assistant::Chosen::ChatGpt { cfg: cfg() };

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
    }
}

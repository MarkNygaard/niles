//! The assistant: an agent on the household's own subscription — Claude
//! Code on a Claude one, pi on a ChatGPT one — answering where Niles's
//! fast model would build something badly or not at all.
//!
//! Both are agents, not APIs, so Niles cannot hand them tools in a
//! request the way it does its own model. Instead Niles serves its tool registry
//! over MCP on a loopback port, and points each run at it with a token
//! that lives for that one run. The token is also how a tool call knows
//! who is typing: it maps to the speaker the run was started for, so
//! `remember_about_me` writes to the right person even though the call
//! arrives on another request. And it is how a background job is held
//! to its own tools: the review cannot turn on a light.
//!
//! What each CLI is asked, and how its answer is read, is in
//! [`crate::claude_code`] and [`crate::chatgpt`]. Everything here is the
//! same for both.

use anyhow::Context;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_config::{Assistant as Preferred, ChatGptConfig, ClaudeCodeConfig, IntegrationsConfig};
use niles_llm::Message;
use niles_tools::ToolRegistry;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One run's grant: who it was started for, and — for a background job
/// — the only tools it may see. `None` is the whole registry.
#[derive(Clone)]
struct Grant {
    speaker: Option<String>,
    tools: Option<Arc<ToolRegistry>>,
    /// Whose turn this run answers, so a tool that waits for a yes holds
    /// its call for them, and a saved artifact is theirs.
    asker: Option<crate::approvals::Asker>,
    /// Which assistant is running: `claude` or `chatgpt`. What a version
    /// it saves is marked as written by.
    via: &'static str,
}

tokio::task_local! {
    /// The assistant whose tool call this is, for the length of the call.
    static VIA: &'static str;
}

/// The assistant making the tool call this runs in, if one is.
pub(crate) fn via() -> Option<&'static str> {
    VIA.try_with(|v| *v).ok()
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// Run `f` as though `via` were making the tool call.
    pub(crate) async fn as_assistant<F: std::future::Future>(via: &'static str, f: F) -> F::Output {
        super::VIA.scope(via, f).await
    }
}

/// Live run tokens, and what each grants.
type Turns = Arc<Mutex<HashMap<String, Grant>>>;

pub(crate) struct Assistant {
    mcp_url: String,
    turns: Turns,
    /// Where a renewed ChatGPT sign-in is kept. Claude's token does not
    /// change, so only ChatGPT needs it.
    secrets: Option<Arc<niles_db::PostgresSecrets>>,
}

/// The assistant chosen for one turn, and what it needs to run.
pub(crate) enum Chosen {
    ClaudeCode {
        cfg: ClaudeCodeConfig,
        token: String,
    },
    ChatGpt {
        cfg: ChatGptConfig,
    },
}

impl Chosen {
    /// How a reply it wrote is marked: `Turn.by`, `Reply.via`.
    pub(crate) fn via(&self) -> &'static str {
        match self {
            Self::ClaudeCode { .. } => "claude",
            Self::ChatGpt { .. } => "chatgpt",
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::ClaudeCode { .. } => "Claude Code",
            Self::ChatGpt { .. } => "ChatGPT",
        }
    }
}

/// Who answers, read per turn: switching one on in the app takes hold
/// with the next message, not the next restart.
pub(crate) enum Choice {
    /// Neither is added and switched on.
    Off,
    Use(Chosen),
    /// One is switched on and cannot run: no token, not signed in. Said
    /// rather than skipped, because falling back quietly made a broken
    /// setup look like one that worked and was merely fast.
    Unready {
        via: &'static str,
        reason: String,
    },
}

pub(crate) fn choose(integrations: &IntegrationsConfig) -> Choice {
    let claude = integrations
        .claude_code
        .as_ref()
        .filter(|c| c.enabled)
        .map(|cfg| {
            cfg.resolve_oauth_token()
                .map(|token| Chosen::ClaudeCode {
                    cfg: cfg.clone(),
                    token,
                })
                .map_err(|e| ("claude", format!("Claude Code has no token: {e}")))
        });
    let chatgpt = integrations
        .chatgpt
        .as_ref()
        .filter(|c| c.enabled)
        .map(|cfg| {
            niles_config::ChatGptSignIn::stored()
                .map(|_| Chosen::ChatGpt { cfg: cfg.clone() })
                .ok_or(("chatgpt", "ChatGPT is not signed in".to_string()))
        });
    pick(integrations.assistant, claude, chatgpt)
}

type Candidate = Option<Result<Chosen, (&'static str, String)>>;

/// The preferred one when it can run, else the other when it can, else
/// why the one that was meant to could not. `None` is not switched on.
///
/// Every gate asks this — the chat, a spoken rule, the fast model's
/// hand-off and the background review — so "neither" chosen here is
/// neither everywhere.
fn pick(preferred: Preferred, claude: Candidate, chatgpt: Candidate) -> Choice {
    let (first, second) = match preferred {
        Preferred::ClaudeCode => (claude, chatgpt),
        Preferred::Chatgpt => (chatgpt, claude),
        Preferred::None => return Choice::Off,
    };
    match (first, second) {
        (Some(Ok(chosen)), _) | (_, Some(Ok(chosen))) => Choice::Use(chosen),
        (Some(Err((via, reason))), _) | (None, Some(Err((via, reason)))) => {
            Choice::Unready { via, reason }
        }
        (None, None) => Choice::Off,
    }
}

impl Assistant {
    /// Serve the tools on a loopback port, so only a process in this pod
    /// can reach them — and only with a run's token.
    ///
    /// `own` are the tools only an assistant is offered, beside the
    /// house's: the artifacts, which the fast model never sees.
    pub(crate) async fn start(
        tools: Arc<ToolRegistry>,
        own: ToolRegistry,
        secrets: Option<Arc<niles_db::PostgresSecrets>>,
    ) -> anyhow::Result<Self> {
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
                own: Arc::new(own),
                turns: turns.clone(),
            });
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("[assistant] the MCP server stopped: {e}");
            }
        });
        Ok(Self {
            mcp_url: format!("http://{addr}/mcp"),
            turns,
            secrets,
        })
    }

    /// Answer one message, in the conversation so far.
    pub(crate) async fn answer(
        &self,
        chosen: &Chosen,
        system_prompt: &str,
        history: &[Message],
        text: &str,
        speaker: Option<String>,
    ) -> anyhow::Result<String> {
        let turn = TurnToken::issue(
            &self.turns,
            Grant {
                speaker,
                tools: None,
                asker: crate::approvals::asker(),
                via: chosen.via(),
            },
        );
        let prompt = transcript(history, text);
        match chosen {
            Chosen::ClaudeCode { cfg, token } => {
                crate::claude_code::run(
                    crate::claude_code::Run::chat(cfg),
                    token,
                    &self.mcp_url,
                    &turn.0,
                    system_prompt,
                    &prompt,
                )
                .await
            }
            Chosen::ChatGpt { cfg } => {
                crate::chatgpt::run(
                    crate::chatgpt::Run::chat(cfg),
                    self.secrets.as_deref(),
                    &self.mcp_url,
                    &turn.0,
                    system_prompt,
                    &prompt,
                )
                .await
            }
        }
    }

    /// A job nobody waits for, with only `tools` to work with: the
    /// house's devices are not among them. `for_whom` is the person a
    /// job is done for, whose own tools it then reaches; `None` for the
    /// household's.
    pub(crate) async fn background(
        &self,
        chosen: &Chosen,
        system_prompt: &str,
        prompt: &str,
        tools: ToolRegistry,
        for_whom: Option<crate::approvals::Asker>,
    ) -> anyhow::Result<String> {
        let turn = TurnToken::issue(
            &self.turns,
            Grant {
                speaker: for_whom.as_ref().and_then(|a| a.slug.clone()),
                tools: Some(Arc::new(tools)),
                asker: for_whom,
                via: chosen.via(),
            },
        );
        match chosen {
            Chosen::ClaudeCode { token, .. } => {
                crate::claude_code::run(
                    crate::claude_code::Run::background(),
                    token,
                    &self.mcp_url,
                    &turn.0,
                    system_prompt,
                    prompt,
                )
                .await
            }
            Chosen::ChatGpt { .. } => {
                crate::chatgpt::run(
                    crate::chatgpt::Run::background(),
                    self.secrets.as_deref(),
                    &self.mcp_url,
                    &turn.0,
                    system_prompt,
                    prompt,
                )
                .await
            }
        }
    }
}

/// A token for one run, withdrawn when the run is over however it ends.
struct TurnToken(String, Turns);

impl TurnToken {
    fn issue(turns: &Turns, grant: Grant) -> Self {
        let token = format!("{:032x}", rand::random::<u128>());
        turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.clone(), grant);
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
    own: Arc<ToolRegistry>,
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
    let grant = token.and_then(|t| {
        state
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(t)
            .cloned()
    });
    let Some(grant) = grant else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // A background job lists and calls only its own tools: asking for
    // `set_device` by name finds nothing.
    let registries: Vec<Arc<ToolRegistry>> = match grant.tools {
        Some(own) => vec![own],
        None => vec![state.tools.clone(), state.own.clone()],
    };
    let handled = VIA.scope(
        grant.via,
        crate::profile::SPEAKER.scope(grant.speaker, async {
            let registries: Vec<&ToolRegistry> = registries.iter().map(|r| r.as_ref()).collect();
            match grant.asker {
                Some(asker) => {
                    crate::approvals::ASKER
                        .scope(asker, niles_tools::mcp::handle(&registries, &message))
                        .await
                }
                None => niles_tools::mcp::handle(&registries, &message).await,
            }
        }),
    );
    match handled.await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude() -> Candidate {
        Some(Ok(Chosen::ClaudeCode {
            cfg: toml::from_str("").unwrap(),
            token: "t".into(),
        }))
    }

    fn chatgpt() -> Candidate {
        Some(Ok(Chosen::ChatGpt {
            cfg: toml::from_str("").unwrap(),
        }))
    }

    fn unready(via: &'static str) -> Candidate {
        Some(Err((via, format!("{via} cannot run"))))
    }

    fn used(choice: Choice) -> Option<&'static str> {
        match choice {
            Choice::Use(chosen) => Some(chosen.via()),
            _ => None,
        }
    }

    #[test]
    fn only_claude_is_claude() {
        assert_eq!(
            used(pick(Preferred::Chatgpt, claude(), None)),
            Some("claude")
        );
    }

    #[test]
    fn only_chatgpt_is_chatgpt_whatever_the_preference() {
        assert_eq!(
            used(pick(Preferred::ClaudeCode, None, chatgpt())),
            Some("chatgpt")
        );
    }

    #[test]
    fn with_both_the_preference_decides() {
        assert_eq!(
            used(pick(Preferred::ClaudeCode, claude(), chatgpt())),
            Some("claude")
        );
        assert_eq!(
            used(pick(Preferred::Chatgpt, claude(), chatgpt())),
            Some("chatgpt")
        );
    }

    #[test]
    fn the_preferred_one_without_credentials_gives_way() {
        assert_eq!(
            used(pick(Preferred::Chatgpt, claude(), unready("chatgpt"))),
            Some("claude")
        );
        assert_eq!(
            used(pick(Preferred::ClaudeCode, unready("claude"), chatgpt())),
            Some("chatgpt")
        );
    }

    #[test]
    fn neither_able_to_run_says_why_the_preferred_one_could_not() {
        match pick(Preferred::Chatgpt, unready("claude"), unready("chatgpt")) {
            Choice::Unready { via, reason } => {
                assert_eq!(via, "chatgpt");
                assert!(reason.contains("cannot run"), "{reason}");
            }
            _ => panic!("expected it to say why"),
        }
        // Not switched on, so not the one to blame.
        match pick(Preferred::Chatgpt, unready("claude"), None) {
            Choice::Unready { via, .. } => assert_eq!(via, "claude"),
            _ => panic!("expected it to say why"),
        }
    }

    #[test]
    fn choosing_neither_keeps_both_out_even_when_both_could_run() {
        assert!(matches!(
            pick(Preferred::None, claude(), chatgpt()),
            Choice::Off
        ));
        // Not "unready" either: nothing is broken, so nothing is said.
        assert!(matches!(
            pick(Preferred::None, unready("claude"), unready("chatgpt")),
            Choice::Off
        ));
    }

    #[test]
    fn choosing_neither_in_the_config_turns_the_assistant_off() {
        unsafe { std::env::set_var("NILES_TEST_ASSISTANT_NONE_TOKEN", "t") };
        let cfg: IntegrationsConfig = toml::from_str(
            "assistant = \"none\"
[claude_code]
oauth_token_env = \"NILES_TEST_ASSISTANT_NONE_TOKEN\"
",
        )
        .unwrap();
        assert!(matches!(choose(&cfg), Choice::Off));
    }

    #[test]
    fn neither_switched_on_is_off() {
        assert!(matches!(
            pick(Preferred::ClaudeCode, None, None),
            Choice::Off
        ));
    }

    #[test]
    fn a_switched_off_one_is_not_chosen() {
        // Without a token anywhere, Claude is switched on and cannot
        // run, and ChatGPT is switched off rather than unready.
        let cfg: IntegrationsConfig = toml::from_str(
            "assistant = \"chatgpt\"\n[claude_code]\noauth_token_env = \"NILES_TEST_NO_SUCH_VAR\"\n[chatgpt]\nenabled = false\n",
        )
        .unwrap();
        match choose(&cfg) {
            Choice::Unready { via, .. } => assert_eq!(via, "claude"),
            _ => panic!("expected Claude to be named"),
        }
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
    fn a_token_is_withdrawn_when_the_turn_ends() {
        let turns: Turns = Arc::default();
        let token = TurnToken::issue(
            &turns,
            Grant {
                speaker: Some("mark".into()),
                tools: None,
                asker: None,
                via: "claude",
            },
        );
        let key = token.0.clone();
        assert_eq!(
            turns.lock().unwrap().get(&key).unwrap().speaker.as_deref(),
            Some("mark")
        );
        drop(token);
        assert!(turns.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_mcp_port_wants_a_live_token() {
        let assistant = Assistant::start(Arc::new(ToolRegistry::new()), ToolRegistry::new(), None)
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

        let refused = client
            .post(&assistant.mcp_url)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);

        let token = TurnToken::issue(
            &assistant.turns,
            Grant {
                speaker: None,
                tools: None,
                asker: None,
                via: "claude",
            },
        );
        let answered = client
            .post(&assistant.mcp_url)
            .bearer_auth(&token.0)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(answered.status(), reqwest::StatusCode::OK);
        let body: Value = answered.json().await.unwrap();
        assert_eq!(body["result"], json!({}));
    }

    fn house() -> ToolRegistry {
        let mut house = ToolRegistry::new();
        niles_tools::register_grocery_tools(
            &mut house,
            Arc::new(niles_groceries::GroceryStore::new()),
            None,
        );
        house
    }

    fn review_tools() -> ToolRegistry {
        let skills = Arc::new(niles_skills::SkillStore::new(100_000));
        let mut own = ToolRegistry::new();
        niles_tools::register_background_skill_tools(&mut own, skills);
        own
    }

    async fn tool_names(url: &str, token: &str) -> Vec<String> {
        let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let body: Value = reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(&list)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    /// A background job's token sees its own tools and none of the
    /// house's: the review must not be able to turn on a light.
    #[tokio::test]
    async fn a_background_grant_sees_only_its_own_tools() {
        let assistant = Assistant::start(Arc::new(house()), ToolRegistry::new(), None)
            .await
            .unwrap();
        let token = TurnToken::issue(
            &assistant.turns,
            Grant {
                speaker: None,
                tools: Some(Arc::new(review_tools())),
                asker: None,
                via: "claude",
            },
        );
        let names = tool_names(&assistant.mcp_url, &token.0).await;
        assert!(names.iter().any(|n| n == "view_skill"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("grocer")), "{names:?}");
    }

    /// The tools only an assistant has (the artifacts) are offered with
    /// the house's in a turn; a background job sees only its own.
    #[tokio::test]
    async fn the_assistant_s_own_tools_join_a_turn_s_but_not_a_job_s() {
        let assistant = Assistant::start(Arc::new(house()), review_tools(), None)
            .await
            .unwrap();
        let turn = TurnToken::issue(
            &assistant.turns,
            Grant {
                speaker: None,
                tools: None,
                asker: None,
                via: "claude",
            },
        );
        let names = tool_names(&assistant.mcp_url, &turn.0).await;
        assert!(names.iter().any(|n| n == "view_skill"), "{names:?}");
        assert!(names.iter().any(|n| n.contains("grocer")), "{names:?}");

        let job = TurnToken::issue(
            &assistant.turns,
            Grant {
                speaker: None,
                tools: Some(Arc::new(ToolRegistry::new())),
                asker: None,
                via: "claude",
            },
        );
        assert!(tool_names(&assistant.mcp_url, &job.0).await.is_empty());
    }

    /// pi reaches the tools through the variables Niles hands it, the
    /// way `deploy/pi/niles.ts` does: the same grant holds there, and
    /// ends with the run.
    #[tokio::test]
    async fn pi_is_held_to_the_same_grant() {
        let assistant = Assistant::start(Arc::new(house()), ToolRegistry::new(), None)
            .await
            .unwrap();
        let token = TurnToken::issue(
            &assistant.turns,
            Grant {
                speaker: None,
                tools: Some(Arc::new(review_tools())),
                asker: None,
                via: "claude",
            },
        );
        let env: HashMap<String, String> = crate::chatgpt::environment(
            std::path::Path::new("/data/pi"),
            &assistant.mcp_url,
            &token.0,
            |_| None,
        )
        .into_iter()
        .collect();
        let (url, bearer) = (&env["NILES_MCP_URL"], &env["NILES_MCP_TOKEN"]);
        let names = tool_names(url, bearer).await;
        assert!(names.iter().any(|n| n == "view_skill"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("grocer")), "{names:?}");

        let bearer = bearer.clone();
        drop(token);
        let after = reqwest::Client::new()
            .post(url)
            .bearer_auth(&bearer)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(after.status(), reqwest::StatusCode::UNAUTHORIZED);
    }
}

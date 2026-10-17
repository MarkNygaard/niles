//! The tool-calling loop: one turn with the language model, its tools, and its answer.

use crate::review;
use anyhow::Context;
use niles_llm::{ChatRequest, ChatResponse, Message, ToolChoice};
use niles_tools::ToolRegistry;
use serde_json::json;

pub(crate) const MAX_TOOL_ITERATIONS: usize = 5;

/// A minimal abstraction over the chat-completions endpoint so the
/// tool-calling loop is testable without spinning up an HTTP server.
/// `OpenAiClient` is the only real implementor; tests use a fake.
#[async_trait::async_trait]
pub(crate) trait ChatProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse>;
}

#[async_trait::async_trait]
impl<T: niles_llm::LlmBackend> ChatProvider for T {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(niles_llm::LlmBackend::chat(self, req).await?)
    }
}

/// Outcome of a tool-calling chat loop. The loop may complete with a
/// final text response, or abort early when the model requests
/// escalation to Tier 2.
#[derive(Debug)]
pub(crate) enum LoopOutcome {
    Done(String),
    EscalateRequested {
        reason: String,
        messages: Vec<Message>,
    },
    /// The model reached for a tool that is handed on rather than run:
    /// nothing from that response was executed.
    HandOff {
        tool: String,
    },
}

/// The tools that change a household's rules. The fast model knows an
/// automation when it hears one — it reached for these every time the
/// curtain reminder came up — but builds them badly, so when an
/// assistant is on, reaching for one hands the whole turn to it instead.
pub(crate) const AUTOMATION_CHANGES: &[&str] = &[
    "propose_automation",
    "create_automation",
    "pause_automation",
    "remove_automation",
];

/// Internal helper that starts from an existing message history.
#[expect(
    clippy::too_many_lines,
    reason = "the tool-calling loop; its exits (done, hand-off, escalate) share the message history"
)]
pub(crate) async fn run_tool_calling_chat_with_messages(
    client: &dyn ChatProvider,
    registry: &ToolRegistry,
    mut messages: Vec<Message>,
    max_iterations: usize,
    exclude_tool: Option<&str>,
    hand_off: &[&str],
) -> anyhow::Result<(LoopOutcome, Vec<review::ToolTrace>)> {
    // Only the tools this request could plausibly use. Every schema
    // goes on the wire on every iteration, and with thirty-odd
    // registered that is most of the prompt — one measured call was
    // 3 497 tokens of an 8 000-per-minute budget.
    //
    // Taken from the last thing the user said rather than passed in,
    // so every caller of this loop benefits without changing.
    let transcript = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::User { content } => Some(content.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    let mut llm_tools = registry.llm_tools_for(transcript);
    if let Some(name) = exclude_tool {
        llm_tools.retain(|t| t.name != name);
    }
    tracing::debug!(
        "sending {} of {} tools for {transcript:?}",
        llm_tools.len(),
        registry.llm_tools().len()
    );
    let mut trace: Vec<review::ToolTrace> = Vec::new();

    for _ in 0..max_iterations {
        let req = ChatRequest {
            messages: messages.clone(),
            tools: Some(llm_tools.clone()),
            tool_choice: Some(ToolChoice::Auto),
        };
        let resp = client.chat(req).await.context("calling LLM")?;

        if resp.tool_calls.is_empty() {
            return Ok((LoopOutcome::Done(resp.content.unwrap_or_default()), trace));
        }

        if let Some(call) = resp
            .tool_calls
            .iter()
            .find(|c| hand_off.contains(&c.name.as_str()))
        {
            return Ok((
                LoopOutcome::HandOff {
                    tool: call.name.clone(),
                },
                trace,
            ));
        }

        messages.push(Message::Assistant {
            content: None,
            tool_calls: Some(resp.tool_calls.clone()),
        });

        let escalate_reason = resp
            .tool_calls
            .iter()
            .find(|c| c.name == niles_tools::escalate::ESCALATE_TOOL_NAME)
            .and_then(|c| c.arguments.get("reason").and_then(|v| v.as_str()))
            .map(String::from);

        if let Some(reason) = escalate_reason {
            tracing::info!("Tier 1 requested escalation; skipping non-escalation tool execution");
            for call in &resp.tool_calls {
                let result = if call.name == niles_tools::escalate::ESCALATE_TOOL_NAME {
                    call.arguments.clone()
                } else {
                    json!({
                        "skipped": "deferred_to_tier2",
                        "reason": "tier1 requested escalation in this turn"
                    })
                };
                trace.push(review::ToolTrace {
                    tool: call.name.clone(),
                    arguments: call.arguments.clone(),
                    result: result.clone(),
                });
                messages.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content: result.to_string(),
                });
            }
            return Ok((LoopOutcome::EscalateRequested { reason, messages }, trace));
        }

        for call in &resp.tool_calls {
            let arg_keys: Vec<&str> = call
                .arguments
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            tracing::info!(
                "tool_call {name}({keys}) [id={id}]",
                id = call.id,
                name = call.name,
                keys = arg_keys.join(",")
            );
            // The whole rule, for automations: a rule the model got wrong
            // can only be put right once it can be seen what it sent.
            if call.name.contains("automation") {
                tracing::info!("tool_call {} arguments: {}", call.name, call.arguments);
            }

            let result = if call.name == niles_tools::escalate::ESCALATE_TOOL_NAME {
                call.arguments.clone()
            } else {
                match registry.execute(call).await {
                    Ok(v) => v,
                    Err(e) => json!({ "error": format!("{e}") }),
                }
            };
            tracing::info!(
                "tool_result {name} -> {body}",
                name = call.name,
                body = result.to_string()
            );
            trace.push(review::ToolTrace {
                tool: call.name.clone(),
                arguments: call.arguments.clone(),
                result: result.clone(),
            });
            messages.push(Message::Tool {
                tool_call_id: call.id.clone(),
                content: result.to_string(),
            });
        }
    }

    Err(anyhow::anyhow!(
        "chat tool-calling loop exhausted after {max_iterations} iterations without a final text answer"
    ))
}

/// Drive a chat conversation with tool calling until the model emits
/// a final text response or `max_iterations` is exhausted.
///
/// Each iteration sends the full message history to `client`. If the
/// response carries no tool calls, the function returns the assistant
/// content (or empty string if `content` is `None`). Otherwise the
/// tool calls are dispatched through `registry` and their results are
/// fed back as `Message::Tool` for the next iteration.
pub(crate) async fn run_tool_calling_chat(
    client: &dyn ChatProvider,
    registry: &ToolRegistry,
    prompt: &str,
    system_prompt: Option<&str>,
    max_iterations: usize,
) -> anyhow::Result<(LoopOutcome, Vec<review::ToolTrace>)> {
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(Message::System {
            content: sys.to_string(),
        });
    }
    messages.push(Message::User {
        content: prompt.to_string(),
    });
    run_tool_calling_chat_with_messages(client, registry, messages, max_iterations, None, &[]).await
}

#[cfg(test)]
mod chat_loop_tests {
    use super::*;
    use niles_llm::{FinishReason, ToolCall};
    use niles_tools::tool::{Tool, ToolDescriptor};
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct FakeChat {
        responses: Mutex<VecDeque<ChatResponse>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl FakeChat {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn captured_requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl niles_llm::LlmBackend for FakeChat {
        async fn chat(&self, req: ChatRequest) -> niles_llm::Result<ChatResponse> {
            self.requests.lock().unwrap().push(req);
            let mut q = self.responses.lock().unwrap();
            q.pop_front()
                .ok_or_else(|| niles_llm::Error::InvalidResponse {
                    reason: "FakeChat ran out of canned responses".into(),
                })
        }
    }

    struct StubTool {
        name: String,
        result: Value,
    }

    #[async_trait::async_trait]
    impl Tool for StubTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: self.name.clone(),
                description: "stub".into(),
                parameters: json!({"type":"object","properties":{},"required":[]}),
            }
        }
        async fn execute(&self, _args: Value) -> niles_tools::Result<Value> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn returns_final_text_when_no_tool_calls() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("hello world".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "hi", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "hello world"));
    }

    #[tokio::test]
    async fn dispatches_tool_call_then_returns_final_text() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "stub".into(),
                    arguments: json!({}),
                }],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("the answer is 42".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"value": 42}),
        }));
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "ask the stub", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "the answer is 42"));
    }

    #[tokio::test]
    async fn errors_when_max_iterations_exhausted() {
        let tool_call_response = || ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "loop".into(),
                name: "stub".into(),
                arguments: json!({}),
            }],
            finish_reason: FinishReason::ToolCalls,
        };
        let fake = FakeChat::new(vec![
            tool_call_response(),
            tool_call_response(),
            tool_call_response(),
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        let err = run_tool_calling_chat(&fake, &registry, "go", None, 3)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("exhausted"),
            "error message should mention exhaustion: got {msg}"
        );
    }

    #[tokio::test]
    async fn a_tool_that_is_handed_off_is_not_run() {
        // "every weekday at 6:30 turn on the bedroom light": the model
        // reaches for the rule tools, and the turn goes to the assistant rather
        // than having the fast model build the rule itself.
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "a".into(),
                name: "propose_automation".into(),
                arguments: json!({}),
            }],
            finish_reason: FinishReason::ToolCalls,
        }]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "propose_automation".into(),
            result: json!({"ok": true}),
        }));
        let messages = vec![Message::User {
            content: "every weekday at 6:30 turn on the bedroom light".into(),
        }];
        let (outcome, trace) = run_tool_calling_chat_with_messages(
            &fake,
            &registry,
            messages,
            5,
            None,
            AUTOMATION_CHANGES,
        )
        .await
        .unwrap();
        assert!(
            matches!(outcome, LoopOutcome::HandOff { ref tool } if tool == "propose_automation")
        );
        assert!(trace.is_empty(), "nothing was run");
        assert_eq!(fake.captured_requests().len(), 1);
    }

    #[tokio::test]
    async fn empty_content_returns_empty_string() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "hi", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s.is_empty()));
    }

    #[tokio::test]
    async fn tool_error_is_passed_back_as_json_error_message() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "boom".into(),
                    name: "ghost".into(),
                    arguments: json!({}),
                }],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("ok we recovered".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "ok we recovered"));
    }

    #[tokio::test]
    async fn trace_records_tool_calls() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![
                    ToolCall {
                        id: "c1".into(),
                        name: "stub".into(),
                        arguments: json!({"x": 1}),
                    },
                    ToolCall {
                        id: "c2".into(),
                        name: "stub".into(),
                        arguments: json!({"x": 2}),
                    },
                ],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("done".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        let (_outcome, trace) = run_tool_calling_chat(&fake, &registry, "multi", None, 5)
            .await
            .unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].tool, "stub");
        assert_eq!(trace[0].arguments, json!({"x": 1}));
        assert_eq!(trace[0].result, json!({"ok": true}));
        assert_eq!(trace[1].tool, "stub");
        assert_eq!(trace[1].arguments, json!({"x": 2}));
    }

    #[tokio::test]
    async fn system_prompt_is_prepended_to_messages() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        run_tool_calling_chat(&fake, &registry, "hello", Some("YOU ARE NILES"), 5)
            .await
            .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        match reqs[0].messages.as_slice() {
            [Message::System { content }, Message::User { content: user }] => {
                assert_eq!(content, "YOU ARE NILES");
                assert_eq!(user, "hello");
            }
            other => panic!("expected [System, User], got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_system_message_when_system_prompt_is_none() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        run_tool_calling_chat(&fake, &registry, "hello", None, 5)
            .await
            .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        match reqs[0].messages.as_slice() {
            [Message::User { content }] => assert_eq!(content, "hello"),
            other => panic!("expected [User] only, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn escalate_requested_when_model_calls_escalate_tool() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
                arguments: json!({"reason": "too hard"}),
            }],
            finish_reason: FinishReason::ToolCalls,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert!(
            matches!(outcome, LoopOutcome::EscalateRequested { ref reason, .. } if reason == "too hard"),
            "expected EscalateRequested, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn escalate_mixed_with_other_tools_skips_side_effect_tools() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "stub".into(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "c2".into(),
                    name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
                    arguments: json!({"reason": "complex"}),
                },
            ],
            finish_reason: FinishReason::ToolCalls,
        }]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"value": 1}),
        }));
        let (outcome, trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].tool, "stub");
        assert_eq!(
            trace[0].result,
            json!({
                "skipped": "deferred_to_tier2",
                "reason": "tier1 requested escalation in this turn"
            })
        );
        assert_eq!(trace[1].tool, niles_tools::escalate::ESCALATE_TOOL_NAME);
        assert_eq!(trace[1].result, json!({"reason": "complex"}));
        assert!(
            matches!(outcome, LoopOutcome::EscalateRequested { ref reason, .. } if reason == "complex"),
            "expected EscalateRequested, got {outcome:?}"
        );
        match &outcome {
            LoopOutcome::EscalateRequested { messages, .. } => {
                assert_eq!(messages.len(), 4); // User + Assistant + Tool(stub) + Tool(escalate)
                match &messages[2] {
                    Message::Tool { content, .. } => assert!(
                        content.contains("deferred_to_tier2"),
                        "expected deferred marker in stub tool response, got {content}"
                    ),
                    other => panic!("expected Tool message, got {other:?}"),
                }
            }
            _ => panic!("expected EscalateRequested"),
        }
    }

    #[tokio::test]
    async fn exclude_tool_removes_it_from_llm_tools() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        // Register a fake escalate tool so it appears in llm_tools
        registry.register(Box::new(StubTool {
            name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
            result: json!({"reason": "test"}),
        }));
        let messages = vec![Message::User {
            content: "hi".into(),
        }];
        run_tool_calling_chat_with_messages(
            &fake,
            &registry,
            messages,
            5,
            Some(niles_tools::escalate::ESCALATE_TOOL_NAME),
            &[],
        )
        .await
        .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        let tools = reqs[0].tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "stub");
    }
}

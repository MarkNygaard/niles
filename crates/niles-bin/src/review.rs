//! Background-review fork: re-run a completed turn through a
//! restricted tool set (memory + skills only) to auto-capture
//! learnings.

use niles_memory::MemoryStore;
use niles_skills::SkillSummary;
use serde_json::Value;
use std::sync::Arc;

/// Trace of one tool call made during the main turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolTrace {
    pub tool: String,
    pub arguments: Value,
    pub result: Value,
}

/// Everything the review fork needs to reconstruct the turn.
pub struct ReviewSnapshot {
    pub transcript: String,
    pub spoken_response: String,
    pub tool_trace: Vec<ToolTrace>,
    pub user_memory: Option<String>,
    pub agent_memory: Option<String>,
    pub skill_summaries: Vec<SkillSummary>,
}

/// System prompt for the background-review fork.
pub const SKILL_REVIEW_PROMPT: &str = r#"You are niles's background-review process.

You have access to exactly these tools:
- memory(action, target, content, old_text) – manage USER.md (target=user) and MEMORY.md (target=agent)
- mint_skill(name, description, version, body) – create a new persistent skill
- patch_skill(name, body) – update an existing skill's body
- delete_skill(name, absorbed_into) – remove a skill
- view_skill(name) – inspect a skill's body

Hard rules:
1. Do NOT capture failures, errors, or "sorry" turns. If the main turn failed, do nothing.
2. Do NOT store ephemeral state (timers, device readings, weather). Only durable knowledge.
3. Prefer patch_skill over mint_skill when a similar skill already exists.
4. Prefer doing NOTHING over creating low-value content. Respond with NOTHING if there is nothing durable to capture.
5. Skill bodies should be concise how-to instructions, NOT raw conversation transcripts.
6. Skill names must be short kebab-case (e.g. "morning-routine", "wfh-tuesdays").

If you have nothing to add, patch, delete, or memorialize, respond with the single word NOTHING."#;

/// Format the user message sent to the review fork.
pub fn format_review_user_message(snapshot: &ReviewSnapshot) -> String {
    let mut out = String::new();

    out.push_str("The user said:\n> ");
    out.push_str(&snapshot.transcript);
    out.push('\n');

    out.push_str("\nniles spoke back:\n> ");
    out.push_str(&snapshot.spoken_response);
    out.push('\n');

    out.push_str("\nTool calls during this turn:\n");
    if snapshot.tool_trace.is_empty() {
        out.push_str("(none)\n");
    } else {
        for t in &snapshot.tool_trace {
            out.push_str(&format!(
                "- {}({}) → {}\n",
                t.tool,
                serde_json::to_string(&t.arguments).unwrap_or_default(),
                serde_json::to_string(&t.result).unwrap_or_default()
            ));
        }
    }

    out.push_str("\nCurrent memory:\n");
    match (&snapshot.user_memory, &snapshot.agent_memory) {
        (None, None) => out.push_str("(none)\n"),
        (user, agent) => {
            out.push_str("USER.md:\n");
            out.push_str(user.as_deref().unwrap_or("(none)"));
            out.push_str("\nAGENT.md:\n");
            out.push_str(agent.as_deref().unwrap_or("(none)"));
            out.push('\n');
        }
    }

    out.push_str("\nAvailable skills:\n");
    if snapshot.skill_summaries.is_empty() {
        out.push_str("(none)\n");
    } else {
        for s in &snapshot.skill_summaries {
            out.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
    }

    out
}

/// Spawn a detached background review task.
///
/// Short-circuits (returns a handle that resolves immediately) when
/// both stores are absent.
pub fn spawn_skill_review(
    snapshot: ReviewSnapshot,
    memory_store: Option<Arc<MemoryStore>>,
    skill_store: Option<Arc<niles_skills::SkillStore>>,
    llm: Arc<dyn crate::ChatProvider>,
    home: Arc<niles_config::HomeConfig>,
    max_iters: usize,
) -> tokio::task::JoinHandle<()> {
    if memory_store.is_none() && skill_store.is_none() {
        return tokio::spawn(async {});
    }

    tokio::spawn(async move {
        let tools = niles_tools::restricted_registry_for_review(memory_store, skill_store);
        let system = format!("{}\n\n{}", SKILL_REVIEW_PROMPT, crate::home_context(&home));
        let user_msg = format_review_user_message(&snapshot);

        match crate::run_tool_calling_chat(
            llm.as_ref(),
            &tools,
            &user_msg,
            Some(&system),
            max_iters,
        )
        .await
        {
            Ok(_) => tracing::debug!("skill review completed"),
            Err(e) => tracing::warn!("skill review failed: {e:#}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skill_review_prompt_smoke() {
        assert!(!SKILL_REVIEW_PROMPT.is_empty());
        assert!(SKILL_REVIEW_PROMPT.contains("memory("));
        assert!(SKILL_REVIEW_PROMPT.contains("mint_skill("));
    }

    #[test]
    fn format_review_user_message_sections_in_order() {
        let snapshot = ReviewSnapshot {
            transcript: "turn on the lights".into(),
            spoken_response: "done".into(),
            tool_trace: vec![],
            user_memory: None,
            agent_memory: None,
            skill_summaries: vec![],
        };
        let msg = format_review_user_message(&snapshot);
        let user_pos = msg.find("The user said:").unwrap();
        let niles_pos = msg.find("niles spoke back:").unwrap();
        let tools_pos = msg.find("Tool calls during this turn:").unwrap();
        let mem_pos = msg.find("Current memory:").unwrap();
        let skills_pos = msg.find("Available skills:").unwrap();
        assert!(user_pos < niles_pos);
        assert!(niles_pos < tools_pos);
        assert!(tools_pos < mem_pos);
        assert!(mem_pos < skills_pos);
    }

    #[test]
    fn format_review_shows_none_for_empty_trace() {
        let snapshot = ReviewSnapshot {
            transcript: "hi".into(),
            spoken_response: "hello".into(),
            tool_trace: vec![],
            user_memory: None,
            agent_memory: None,
            skill_summaries: vec![],
        };
        let msg = format_review_user_message(&snapshot);
        assert!(msg.contains("Tool calls during this turn:\n(none)"));
    }

    #[test]
    fn format_review_shows_tool_calls() {
        let snapshot = ReviewSnapshot {
            transcript: "hi".into(),
            spoken_response: "hello".into(),
            tool_trace: vec![ToolTrace {
                tool: "memory".into(),
                arguments: json!({"action": "add"}),
                result: json!({"ok": true}),
            }],
            user_memory: None,
            agent_memory: None,
            skill_summaries: vec![],
        };
        let msg = format_review_user_message(&snapshot);
        assert!(msg.contains("memory("));
        assert!(msg.contains("→"));
    }

    #[test]
    fn format_review_shows_none_for_absent_memory() {
        let snapshot = ReviewSnapshot {
            transcript: "hi".into(),
            spoken_response: "hello".into(),
            tool_trace: vec![],
            user_memory: None,
            agent_memory: None,
            skill_summaries: vec![],
        };
        let msg = format_review_user_message(&snapshot);
        assert!(msg.contains("Current memory:\n(none)"));
    }

    #[test]
    fn format_review_shows_memory_sections() {
        let snapshot = ReviewSnapshot {
            transcript: "hi".into(),
            spoken_response: "hello".into(),
            tool_trace: vec![],
            user_memory: Some("User likes tea.".into()),
            agent_memory: Some("Agent learned Danish.".into()),
            skill_summaries: vec![],
        };
        let msg = format_review_user_message(&snapshot);
        assert!(msg.contains("USER.md:\nUser likes tea."));
        assert!(msg.contains("AGENT.md:\nAgent learned Danish."));
    }

    #[test]
    fn format_review_shows_skill_summaries() {
        let snapshot = ReviewSnapshot {
            transcript: "hi".into(),
            spoken_response: "hello".into(),
            tool_trace: vec![],
            user_memory: None,
            agent_memory: None,
            skill_summaries: vec![SkillSummary {
                name: "lights".into(),
                description: "Control lights".into(),
                version: "1.0.0".into(),
                status: niles_skills::SkillStatus::Active,
                last_activity_at: None,
                pinned: false,
                provenance: niles_skills::Provenance::UserCreated,
            }],
        };
        let msg = format_review_user_message(&snapshot);
        assert!(msg.contains("- lights: Control lights"));
    }
}

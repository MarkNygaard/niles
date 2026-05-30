//! Tier 2 escalation tool.
//!
//! When the Tier 1 model calls `escalate_to_tier2`, the dispatch
//! layer detects it at the control-flow level (not inside the tool)
//! and re-runs the turn through the Tier 2 backend. The tool itself
//! is a no-op — it simply echoes its arguments back so the model
//! receives a valid tool result.

use crate::error::Result;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use serde_json::Value;

/// Name exposed to the LLM.
pub const ESCALATE_TOOL_NAME: &str = "escalate_to_tier2";

/// Tool that signals a request should be handed off to Tier 2.
pub struct EscalateToTier2Tool;

#[async_trait]
impl Tool for EscalateToTier2Tool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: ESCALATE_TOOL_NAME.into(),
            description: "Escalate the current request to Tier 2 for more capable processing."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why the request needs Tier 2 escalation"
                    }
                },
                "required": ["reason"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        // No-op: escalation is control flow detected by the dispatch
        // layer, not side-effects inside the tool.
        Ok(args)
    }
}

/// Register the escalation tool in a registry.
pub fn register_escalate_tool(registry: &mut crate::registry::ToolRegistry) {
    registry.register(Box::new(EscalateToTier2Tool));
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn descriptor_has_expected_shape() {
        let tool = EscalateToTier2Tool;
        let desc = tool.descriptor();
        assert_eq!(desc.name, "escalate_to_tier2");
        assert!(desc.description.contains("Tier 2"));
        let params = desc.parameters;
        assert_eq!(params["type"], "object");
        assert!(params["properties"].get("reason").is_some());
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&json!("reason")));
    }

    #[tokio::test]
    async fn execute_echoes_arguments() {
        let tool = EscalateToTier2Tool;
        let args = json!({"reason": "complex math"});
        let result = tool.execute(args.clone()).await.unwrap();
        assert_eq!(result, args);
    }

    #[test]
    fn to_llm_tool_round_trips() {
        let tool = EscalateToTier2Tool;
        let llm_tool = tool.descriptor().to_llm_tool();
        assert_eq!(llm_tool.name, "escalate_to_tier2");
        assert_eq!(
            llm_tool.description,
            "Escalate the current request to Tier 2 for more capable processing."
        );
    }
}

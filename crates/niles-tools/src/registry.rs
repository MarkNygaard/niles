//! Tool registry — dispatches LLM tool calls to async handlers.

use crate::error::{Error, Result};
use crate::tool::Tool;
use std::collections::HashMap;

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. If a tool with the same name already exists,
    /// it is replaced (last writer wins).
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.descriptor().name;
        self.tools.insert(name, tool);
    }

    /// All registered tools, as `niles_llm::Tool` wire types ready to
    /// send in a `ChatRequest`.
    pub fn llm_tools(&self) -> Vec<niles_llm::Tool> {
        let mut out: Vec<niles_llm::Tool> = self
            .tools
            .values()
            .map(|t| t.descriptor().to_llm_tool())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Dispatch a `ToolCall`. Returns the JSON value the tool produced;
    /// the caller wraps it in `Message::Tool { tool_call_id, content }`.
    pub async fn execute(&self, call: &niles_llm::ToolCall) -> Result<serde_json::Value> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| Error::UnknownTool(call.name.clone()))?;
        tool.execute(call.arguments.clone()).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolDescriptor;
    use async_trait::async_trait;
    use serde_json::{Value, json};

    struct StubTool {
        name: String,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: self.name.clone(),
                description: "a stub".into(),
                parameters: json!({"type":"object","properties":{},"required":[]}),
            }
        }

        async fn execute(&self, _args: Value) -> Result<Value> {
            Ok(json!({"ok": 1}))
        }
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_unknown_tool_error() {
        let reg = ToolRegistry::new();
        let call = niles_llm::ToolCall {
            id: "x".into(),
            name: "ghost".into(),
            arguments: json!({}),
        };
        let err = reg.execute(&call).await.unwrap_err();
        assert!(matches!(err, Error::UnknownTool(name) if name == "ghost"));
    }

    #[tokio::test]
    async fn execute_dispatches_to_registered_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(StubTool {
            name: "stub".into(),
        }));
        let call = niles_llm::ToolCall {
            id: "c1".into(),
            name: "stub".into(),
            arguments: json!({}),
        };
        let result = reg.execute(&call).await.unwrap();
        assert_eq!(result["ok"], 1);
    }

    #[test]
    fn llm_tools_round_trips_descriptors_to_wire() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(StubTool {
            name: "stub".into(),
        }));
        let tools = reg.llm_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "stub");
        assert_eq!(tools[0].description, "a stub");
        let params = &tools[0].parameters;
        assert_eq!(params["type"], "object");
    }

    #[test]
    fn llm_tools_are_sorted_by_name() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(StubTool { name: "z".into() }));
        reg.register(Box::new(StubTool { name: "a".into() }));
        reg.register(Box::new(StubTool { name: "m".into() }));

        let names: Vec<String> = reg.llm_tools().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }
}

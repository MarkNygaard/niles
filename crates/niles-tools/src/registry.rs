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

    /// The tools worth sending for `transcript`.
    ///
    /// Every schema here goes on the wire on every call, so a registry
    /// this size is most of the prompt. Sending only what the request
    /// could plausibly use is the difference between two questions a
    /// minute and rather more — see [`crate::relevance`], which also
    /// explains why an unclassifiable request still gets everything.
    pub fn llm_tools_for(&self, transcript: &str) -> Vec<niles_llm::Tool> {
        let Some(wanted) = crate::relevance::relevant_tool_names(transcript) else {
            return self.llm_tools();
        };
        let mut out: Vec<niles_llm::Tool> = self
            .tools
            .values()
            .map(|t| t.descriptor().to_llm_tool())
            // A tool nobody classified is a tool that still has to
            // work, so an unknown name keeps its place.
            .filter(|t| {
                wanted.binary_search(&t.name.as_str()).is_ok()
                    || !crate::relevance::is_grouped(&t.name)
            })
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

#[cfg(test)]
mod gating_tests {
    use super::*;
    use crate::tool::{Tool, ToolDescriptor};

    struct Named(&'static str);

    #[async_trait::async_trait]
    impl Tool for Named {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: self.0.into(),
                description: "x".into(),
                parameters: serde_json::json!({"type": "object"}),
            }
        }
        async fn execute(&self, _args: serde_json::Value) -> crate::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    fn registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        for name in [
            "set_device",
            "get_weather",
            "list_timers",
            "some_new_tool_nobody_classified",
        ] {
            reg.register(Box::new(Named(name)));
        }
        reg
    }

    #[test]
    fn an_unrelated_tool_stays_off_the_wire() {
        let names: Vec<String> = registry()
            .llm_tools_for("what's the weather like")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(names.contains(&"get_weather".to_string()));
        assert!(!names.contains(&"list_timers".to_string()));
    }

    #[test]
    fn a_tool_nobody_classified_is_always_sent() {
        // Forgetting to group a new tool must cost a saving, never a
        // capability — otherwise it fails silently, as "the model just
        // said it couldn't".
        let names: Vec<String> = registry()
            .llm_tools_for("what's the weather like")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(names.contains(&"some_new_tool_nobody_classified".to_string()));
    }

    #[test]
    fn an_unclassifiable_request_still_gets_everything() {
        let all = registry().llm_tools().len();
        assert_eq!(registry().llm_tools_for("do the thing").len(), all);
    }
}

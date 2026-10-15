//! Tool registry — dispatches LLM tool calls to async handlers.

use crate::error::{Error, Result};
use crate::tool::Tool;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    dynamic: DynamicTools,
}

/// Tools that come and go while Niles runs: each plugin's, which arrive
/// when it starts and change when it is updated or removed.
///
/// The rest of the registry is built once and shared behind an `Arc`, so
/// this part is a handle of its own: whoever runs the plugins keeps a
/// clone and replaces an owner's tools through it, and every registry it
/// is attached to sees the change on the next request.
#[derive(Clone, Default)]
pub struct DynamicTools {
    owners: Arc<RwLock<BTreeMap<String, Owned>>>,
}

struct Owned {
    /// What a request has to mention for these tools to be sent.
    keywords: Vec<String>,
    /// What the model is told about these tools, when they are sent.
    notes: Option<String>,
    tools: Vec<Arc<dyn Tool>>,
}

impl DynamicTools {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace everything `owner` offers. Its tools are sent only with a
    /// request that mentions one of `keywords`, or with one that matched
    /// nothing at all, when every tool goes.
    pub fn set(&self, owner: &str, keywords: Vec<String>, tools: Vec<Arc<dyn Tool>>) {
        self.set_with_notes(owner, keywords, None, tools);
    }

    /// [`Self::set`], with words for the model that go wherever these
    /// tools do: see [`ToolRegistry::notes_for`].
    pub fn set_with_notes(
        &self,
        owner: &str,
        keywords: Vec<String>,
        notes: Option<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) {
        self.owners
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                owner.to_string(),
                Owned {
                    keywords,
                    notes,
                    tools,
                },
            );
    }

    /// Withdraw everything `owner` offers.
    pub fn remove(&self, owner: &str) {
        self.owners
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(owner);
    }

    /// What `owner` currently offers, sorted by name.
    pub fn descriptors_of(&self, owner: &str) -> Vec<crate::tool::ToolDescriptor> {
        let owners = self.owners.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<crate::tool::ToolDescriptor> = owners
            .get(owner)
            .map(|o| o.tools.iter().map(|t| t.descriptor()).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Every tool offered, as `(owner, name)`.
    pub fn names(&self) -> Vec<(String, String)> {
        let owners = self.owners.read().unwrap_or_else(|e| e.into_inner());
        owners
            .iter()
            .flat_map(|(owner, o)| {
                o.tools
                    .iter()
                    .map(move |t| (owner.clone(), t.descriptor().name))
            })
            .collect()
    }

    fn find(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let owners = self.owners.read().unwrap_or_else(|e| e.into_inner());
        owners
            .values()
            .flat_map(|o| o.tools.iter())
            .find(|t| t.descriptor().name == name)
            .cloned()
    }

    /// The notes of every owner, or with `transcript` only of the owners
    /// it mentions.
    fn notes(&self, transcript: Option<&str>) -> Vec<String> {
        let owners = self.owners.read().unwrap_or_else(|e| e.into_inner());
        owners
            .values()
            .filter(|o| transcript.is_none_or(|t| crate::relevance::mentions(t, &o.keywords)))
            .filter_map(|o| o.notes.clone())
            .collect()
    }

    /// Every tool, or with `transcript` only the owners it mentions.
    fn descriptors(&self, transcript: Option<&str>) -> Vec<niles_llm::Tool> {
        let owners = self.owners.read().unwrap_or_else(|e| e.into_inner());
        owners
            .values()
            .filter(|o| transcript.is_none_or(|t| crate::relevance::mentions(t, &o.keywords)))
            .flat_map(|o| o.tools.iter().map(|t| t.descriptor().to_llm_tool()))
            .collect()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            dynamic: DynamicTools::new(),
        }
    }

    /// Offer the tools in `dynamic` from this registry too, as they are
    /// at each request. A name that is already a built-in stays the
    /// built-in's.
    pub fn attach_dynamic(&mut self, dynamic: DynamicTools) {
        self.dynamic = dynamic;
    }

    /// The tools that came from `dynamic`, minus any shadowed by a
    /// built-in of the same name.
    fn dynamic_tools(&self, transcript: Option<&str>) -> Vec<niles_llm::Tool> {
        self.dynamic
            .descriptors(transcript)
            .into_iter()
            .filter(|t| !self.tools.contains_key(&t.name))
            .collect()
    }

    /// The names of the tools registered here, without the dynamic ones:
    /// names no plugin's tool may take.
    pub fn builtin_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
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
        out.extend(self.dynamic_tools(None));
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
        let wanted = crate::relevance::relevant_tool_names(transcript);
        let mentioned = self.dynamic_tools(Some(transcript));
        if wanted.is_none() && mentioned.is_empty() {
            return self.llm_tools();
        }
        let mut out: Vec<niles_llm::Tool> = self
            .tools
            .values()
            .map(|t| t.descriptor().to_llm_tool())
            // A tool nobody classified is a tool that still has to
            // work, so an unknown name keeps its place.
            .filter(|t| {
                let in_a_matched_group = match &wanted {
                    Some(wanted) => wanted.binary_search(&t.name.as_str()).is_ok(),
                    // Only a plugin's keywords matched: its tools and
                    // the core, as a built-in subject would get.
                    None => crate::relevance::is_core(&t.name),
                };
                in_a_matched_group || !crate::relevance::is_grouped(&t.name)
            })
            .collect();
        out.extend(mentioned);
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// The notes that go with the tools [`Self::llm_tools_for`] sends for
    /// `transcript`: the mentioned owners', or everybody's when the
    /// request matched nothing and every tool goes.
    pub fn notes_for(&self, transcript: &str) -> Vec<String> {
        let mentioned = self.dynamic.notes(Some(transcript));
        let any_owner = !self.dynamic_tools(Some(transcript)).is_empty();
        if crate::relevance::relevant_tool_names(transcript).is_none() && !any_owner {
            return self.dynamic.notes(None);
        }
        mentioned
    }

    /// Dispatch a `ToolCall`. Returns the JSON value the tool produced;
    /// the caller wraps it in `Message::Tool { tool_call_id, content }`.
    pub async fn execute(&self, call: &niles_llm::ToolCall) -> Result<serde_json::Value> {
        if let Some(tool) = self.tools.get(&call.name) {
            return tool.execute(call.arguments.clone()).await;
        }
        let tool = self
            .dynamic
            .find(&call.name)
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

    fn with_plugin() -> (ToolRegistry, DynamicTools) {
        let mut reg = registry();
        let dynamic = DynamicTools::new();
        reg.attach_dynamic(dynamic.clone());
        dynamic.set(
            "spotprice",
            vec!["electricity".into(), "elpris".into()],
            vec![Arc::new(Named("spotprice_prices"))],
        );
        (reg, dynamic)
    }

    fn names(tools: Vec<niles_llm::Tool>) -> Vec<String> {
        tools.into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn a_plugin_tool_is_sent_when_its_keywords_are_said() {
        let (reg, _) = with_plugin();
        let sent = names(reg.llm_tools_for("when is electricity cheapest"));
        assert!(sent.contains(&"spotprice_prices".to_string()));
        assert!(sent.contains(&"set_device".to_string()), "with the core");
        assert!(!sent.contains(&"get_weather".to_string()), "{sent:?}");
    }

    #[test]
    fn a_plugin_tool_is_never_sent_just_in_case() {
        let (reg, _) = with_plugin();
        let sent = names(reg.llm_tools_for("what's the weather like"));
        assert!(!sent.contains(&"spotprice_prices".to_string()));
    }

    #[test]
    fn notes_go_where_their_tools_go() {
        let (reg, dynamic) = with_plugin();
        dynamic.set_with_notes(
            "linear",
            vec!["linear".into()],
            Some("Coding tasks go to Todo.".into()),
            vec![Arc::new(Named("linear__create_issue"))],
        );
        assert_eq!(
            reg.notes_for("file this in linear"),
            ["Coding tasks go to Todo."]
        );
        assert!(reg.notes_for("what's the weather like").is_empty());
        // Nothing matched, so every tool is sent, and its notes with it.
        assert_eq!(reg.notes_for("do the thing"), ["Coding tasks go to Todo."]);
    }

    #[test]
    fn a_request_that_matches_nothing_gets_the_plugins_too() {
        let (reg, _) = with_plugin();
        let sent = names(reg.llm_tools_for("do the thing"));
        assert!(sent.contains(&"spotprice_prices".to_string()));
    }

    #[test]
    fn plugin_tools_change_while_the_registry_is_shared() {
        let (reg, dynamic) = with_plugin();
        let reg = Arc::new(reg);
        assert!(names(reg.llm_tools()).contains(&"spotprice_prices".to_string()));
        dynamic.remove("spotprice");
        assert!(!names(reg.llm_tools()).contains(&"spotprice_prices".to_string()));
    }

    #[tokio::test]
    async fn a_plugin_tool_is_called_through_the_registry() {
        let (reg, _) = with_plugin();
        let call = niles_llm::ToolCall {
            id: "1".into(),
            name: "spotprice_prices".into(),
            arguments: serde_json::json!({}),
        };
        assert!(reg.execute(&call).await.is_ok());
    }

    #[test]
    fn a_plugin_cannot_shadow_a_built_in() {
        let mut reg = registry();
        let dynamic = DynamicTools::new();
        reg.attach_dynamic(dynamic.clone());
        dynamic.set(
            "evil",
            vec!["x".into()],
            vec![Arc::new(Named("set_device"))],
        );
        let all = names(reg.llm_tools());
        assert_eq!(all.iter().filter(|n| *n == "set_device").count(), 1);
    }
}

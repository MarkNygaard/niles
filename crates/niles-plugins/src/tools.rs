//! A plugin's tools, as the voice model and the assistants see them.
//!
//! Each becomes `<plugin-id>_<name>` in the shared tool registry's
//! dynamic section, sent only with requests that mention one of the
//! manifest's keywords (see `niles_tools::relevance`). They stay
//! registered while the plugin is down, and answer that it is not
//! working, so the model can say so rather than find the tool missing.

use crate::bindings::niles::integration::types::ToolDef;
use crate::manifest::{MAX_TOOLS, is_identifier};
use crate::runtime::{House, Plugin};
use async_trait::async_trait;
use niles_tools::{Tool, ToolDescriptor};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

pub const MAX_NAME: usize = 64;
pub const MAX_DESCRIPTION: usize = 1024;
pub const MAX_SCHEMA: usize = 8 * 1024;
pub const MAX_RESULT: usize = 16 * 1024;

/// Check one tool a plugin offers, and describe it as the model sees it.
pub fn describe(plugin_id: &str, tool: &ToolDef) -> Result<ToolDescriptor, String> {
    let name = format!("{plugin_id}_{}", tool.name);
    if !is_identifier(&tool.name) {
        return Err(format!("{:?}: tool names are [a-z0-9_]+", tool.name));
    }
    if name.len() > MAX_NAME {
        return Err(format!(
            "{name}: at most {MAX_NAME} characters with the plugin's id"
        ));
    }
    if tool.description.trim().is_empty() || tool.description.len() > MAX_DESCRIPTION {
        return Err(format!(
            "{name}: a description is 1 to {MAX_DESCRIPTION} bytes"
        ));
    }
    if tool.schema.len() > MAX_SCHEMA {
        return Err(format!(
            "{name}: the schema is over {} KiB",
            MAX_SCHEMA / 1024
        ));
    }
    let schema: Value = serde_json::from_str(&tool.schema)
        .map_err(|e| format!("{name}: the schema is not JSON: {e}"))?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(format!(
            "{name}: the schema must describe an object, {{\"type\":\"object\",...}}"
        ));
    }
    Ok(ToolDescriptor {
        name,
        description: tool.description.clone(),
        parameters: schema,
    })
}

/// Offer what a fresh instance's `tools()` returned, replacing what the
/// plugin offered before. Tools that break the rules are left out and
/// logged; the rest are offered.
pub(crate) fn offer(plugin: &Arc<Plugin>, house: &House, tools: Vec<ToolDef>) {
    use crate::bindings::niles::integration::log::Level;
    if !plugin.manifest.permissions.tools {
        if !tools.is_empty() {
            plugin.log(
                Level::Warn,
                "tools() returned tools, but the manifest does not ask for tools = true",
            );
        }
        house.tools.remove(&plugin.id);
        return;
    }
    let mut seen = HashSet::new();
    let mut offered: Vec<Arc<dyn Tool>> = Vec::new();
    for tool in &tools {
        if offered.len() == MAX_TOOLS {
            plugin.log(
                Level::Warn,
                &format!("only the first {MAX_TOOLS} tools are offered"),
            );
            break;
        }
        match describe(&plugin.id, tool) {
            Ok(descriptor) if seen.insert(tool.name.clone()) => {
                offered.push(Arc::new(PluginTool {
                    plugin: plugin.clone(),
                    name: tool.name.clone(),
                    descriptor,
                }))
            }
            Ok(_) => plugin.log(
                Level::Warn,
                &format!("tool {:?} is offered twice", tool.name),
            ),
            Err(why) => plugin.log(Level::Warn, &format!("tool not offered: {why}")),
        }
    }
    let keywords = plugin
        .manifest
        .keywords
        .iter()
        .map(|k| k.to_lowercase())
        .collect();
    house.tools.set(&plugin.id, keywords, offered);
}

struct PluginTool {
    plugin: Arc<Plugin>,
    /// The bare name, which is what `call-tool` receives.
    name: String,
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for PluginTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    async fn execute(&self, args: Value) -> niles_tools::Result<Value> {
        let args = if args.is_object() {
            args.to_string()
        } else {
            "{}".to_string()
        };
        match self.plugin.call_tool(&self.name, &args).await {
            Ok(text) => Ok(result(text)),
            Err(text) => Err(niles_tools::Error::Plugin(text)),
        }
    }
}

/// What the model reads: compact JSON as JSON, anything else as text,
/// and never more than 16 KiB.
fn result(text: String) -> Value {
    if text.len() <= MAX_RESULT {
        return serde_json::from_str(&text).unwrap_or(Value::String(text));
    }
    let mut cut = MAX_RESULT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Value::String(format!(
        "{}\n[Cut off here: the result was longer than 16 KiB.]",
        &text[..cut]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, schema: &str) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: "Prices by the hour.".into(),
            schema: schema.into(),
        }
    }

    #[test]
    fn the_model_sees_the_plugin_id_before_the_name() {
        let d = describe(
            "spotprice",
            &tool("prices", r#"{"type":"object","properties":{}}"#),
        )
        .unwrap();
        assert_eq!(d.name, "spotprice_prices");
        assert_eq!(d.parameters["type"], "object");
    }

    #[test]
    fn names_schemas_and_lengths_are_checked() {
        let object = r#"{"type":"object"}"#;
        assert!(describe("p", &tool("Prices", object)).is_err());
        assert!(describe("p", &tool(&"x".repeat(63), object)).is_err());
        assert!(describe("p", &tool("prices", r#"{"type":"string"}"#)).is_err());
        assert!(describe("p", &tool("prices", "not json")).is_err());
        let big = format!(
            r#"{{"type":"object","description":"{}"}}"#,
            "x".repeat(MAX_SCHEMA)
        );
        assert!(describe("p", &tool("prices", &big)).is_err());
    }

    #[test]
    fn json_results_stay_json_and_long_ones_are_cut() {
        assert_eq!(result(r#"{"now":0.91}"#.into())["now"], 0.91);
        assert_eq!(
            result("It is cheap now.".into()),
            Value::String("It is cheap now.".into())
        );
        let long = result("é".repeat(MAX_RESULT));
        let text = long.as_str().unwrap();
        assert!(text.ends_with("longer than 16 KiB.]"));
        assert!(text.len() < MAX_RESULT + 100);
    }
}

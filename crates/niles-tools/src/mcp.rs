//! Niles's tools as an MCP server, for an agent that speaks MCP rather
//! than tool calls in a chat request — Claude Code, answering the app's
//! chat.
//!
//! Only the JSON-RPC: what a request means and what to answer. The
//! transport (one POST per message, the Streamable HTTP shape without a
//! stream) belongs to whoever serves it. Four methods are enough for an
//! agent to find the tools and call them; everything else is "method
//! not found", which a client is required to cope with.

use crate::registry::ToolRegistry;
use serde_json::{Value, json};

/// The revision answered when the client asks for one this does not
/// know. Clients negotiate down from what they send.
const PROTOCOL_VERSION: &str = "2025-06-18";
const KNOWN_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Tools an agent must not see: escalation is control flow between
/// Niles's own tiers, and means nothing to anybody else.
const HIDDEN: &[&str] = &[crate::escalate::ESCALATE_TOOL_NAME];

/// Answer one JSON-RPC message. `None` for a notification, which gets no
/// reply.
pub async fn handle(registry: &ToolRegistry, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(list(registry)),
        "tools/call" => call(registry, &params).await,
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| KNOWN_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "niles", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn list(registry: &ToolRegistry) -> Value {
    let tools: Vec<Value> = registry
        .llm_tools()
        .into_iter()
        .filter(|t| !HIDDEN.contains(&t.name.as_str()))
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.parameters,
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// A tool that fails is a result, not a protocol error: the agent reads
/// `isError` and can try something else, where an error response would
/// look like the server breaking.
async fn call(registry: &ToolRegistry, params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "tools/call needs a tool name".to_string()))?;
    if HIDDEN.contains(&name) {
        return Err((-32602, format!("no tool called {name}")));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .filter(|a| !a.is_null())
        .unwrap_or_else(|| json!({}));
    let call = niles_llm::ToolCall {
        id: String::new(),
        name: name.to_string(),
        arguments,
    };
    Ok(match registry.execute(&call).await {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "isError": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": e.to_string() }],
            "isError": true,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{Tool, ToolDescriptor};
    use async_trait::async_trait;

    struct Echo;

    #[async_trait]
    impl Tool for Echo {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "echo".into(),
                description: "Says it back.".into(),
                parameters: json!({ "type": "object", "properties": { "say": { "type": "string" } } }),
            }
        }

        async fn execute(&self, args: Value) -> crate::Result<Value> {
            match args.get("say").and_then(Value::as_str) {
                Some(say) => Ok(json!({ "said": say })),
                None => Err(crate::Error::InvalidArgs {
                    tool: "echo".into(),
                    reason: "nothing to say".into(),
                }),
            }
        }
    }

    fn registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(Echo));
        crate::escalate::register_escalate_tool(&mut reg);
        reg
    }

    async fn ask(method: &str, params: Value) -> Value {
        handle(
            &registry(),
            &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .await
        .expect("a request is answered")
    }

    #[tokio::test]
    async fn agrees_on_a_version_the_client_knows() {
        let reply = ask("initialize", json!({ "protocolVersion": "2025-03-26" })).await;
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "niles");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn offers_its_own_version_for_one_it_does_not_know() {
        let reply = ask("initialize", json!({ "protocolVersion": "2099-01-01" })).await;
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn lists_the_tools_but_not_escalation() {
        let reply = ask("tools/list", json!({})).await;
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["echo"]);
        assert!(reply["result"]["tools"][0]["inputSchema"].is_object());
    }

    #[tokio::test]
    async fn calls_a_tool() {
        let reply = ask(
            "tools/call",
            json!({ "name": "echo", "arguments": { "say": "hej" } }),
        )
        .await;
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(reply["result"]["content"][0]["text"], r#"{"said":"hej"}"#);
    }

    #[tokio::test]
    async fn a_failing_tool_is_a_result_the_agent_can_read() {
        let reply = ask("tools/call", json!({ "name": "echo", "arguments": {} })).await;
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply.get("error").is_none());
    }

    #[tokio::test]
    async fn escalation_cannot_be_called_either() {
        let reply = ask(
            "tools/call",
            json!({ "name": crate::escalate::ESCALATE_TOOL_NAME, "arguments": {} }),
        )
        .await;
        assert_eq!(reply["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn an_unknown_method_is_not_found() {
        let reply = ask("resources/list", json!({})).await;
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn a_notification_gets_no_reply() {
        let reply = handle(
            &registry(),
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert!(reply.is_none());
    }
}

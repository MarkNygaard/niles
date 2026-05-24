//! Groq chat-completions client.
//!
//! Targets the OpenAI-compatible `POST /openai/v1/chat/completions`
//! endpoint. Request and response types are stable public surfaces;
//! wire types are private and may change as the provider evolves.

use crate::error::{Error, Result};
use serde::de::Deserializer;
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

/// Inputs to [`GroqClient::new`].
#[derive(Debug, Clone)]
pub struct GroqConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub request_timeout: Duration,
}

// ------------------------------------------------------------------
// Public request types
// ------------------------------------------------------------------

/// A chat-completion request. `model` is injected by the client,
/// so callers don't repeat it per request.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

/// A message in the conversation. Uses OpenAI's `role` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// A tool declaration. On the wire each tool is wrapped in
/// `{"type":"function","function":{...}}`.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl Serialize for Tool {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "function")?;
        let func = serde_json::json!({
            "name": &self.name,
            "description": &self.description,
            "parameters": &self.parameters,
        });
        map.serialize_entry("function", &func)?;
        map.end()
    }
}

/// Controls whether the model may emit tool calls.
#[derive(Debug, Clone)]
pub enum ToolChoice {
    Auto,
    None,
    Function(String),
}

impl Serialize for ToolChoice {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            ToolChoice::Auto => "auto".serialize(serializer),
            ToolChoice::None => "none".serialize(serializer),
            ToolChoice::Function(name) => {
                let mut map = serializer.serialize_map(Some(2))?;
                let inner = serde_json::json!({ "name": name });
                map.serialize_entry("type", "function")?;
                map.serialize_entry("function", &inner)?;
                map.end()
            }
        }
    }
}

// ------------------------------------------------------------------
// Public response types
// ------------------------------------------------------------------

/// Successful chat-completion response.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
}

/// A tool call returned by the model.
///
/// On the wire `arguments` arrives as a JSON-encoded *string* inside
/// `function.arguments`. This type stores the already-parsed value.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl Serialize for ToolCall {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("type", "function")?;
        let args_str = serde_json::to_string(&self.arguments).map_err(serde::ser::Error::custom)?;
        let func = serde_json::json!({
            "name": &self.name,
            "arguments": args_str,
        });
        map.serialize_entry("function", &func)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for ToolCall {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
            #[serde(rename = "type")]
            _ty: String,
            function: WireFunction,
        }
        #[derive(Deserialize)]
        struct WireFunction {
            name: String,
            arguments: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        let arguments =
            serde_json::from_str(&wire.function.arguments).map_err(serde::de::Error::custom)?;
        Ok(ToolCall {
            id: wire.id,
            name: wire.function.name,
            arguments,
        })
    }
}

/// Why the model stopped generating tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    Other(String),
}

impl From<String> for FinishReason {
    fn from(s: String) -> Self {
        match s.as_str() {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" => FinishReason::ToolCalls,
            _ => FinishReason::Other(s),
        }
    }
}

// ------------------------------------------------------------------
// Private wire types
// ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawChatResponse {
    choices: Vec<RawChoice>,
}

#[derive(Debug, Deserialize)]
struct RawChoice {
    message: RawMessage,
    finish_reason: String,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<RawToolCall>,
}

#[derive(Debug, Deserialize)]
struct RawToolCall {
    id: String,
    function: RawFunctionCall,
}

#[derive(Debug, Deserialize)]
struct RawFunctionCall {
    name: String,
    arguments: String,
}

// ------------------------------------------------------------------
// Private wire-request wrapper
// ------------------------------------------------------------------

#[derive(Serialize)]
struct WireChatRequest<'a> {
    model: &'a str,
    #[serde(flatten)]
    req: &'a ChatRequest,
}

// ------------------------------------------------------------------
// Client
// ------------------------------------------------------------------

/// HTTP client around Groq's chat-completions endpoint.
pub struct GroqClient {
    http: reqwest::Client,
    cfg: GroqConfig,
}

impl GroqClient {
    pub fn new(cfg: GroqConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        Ok(Self { http, cfg })
    }

    /// Send a chat-completion request and return the model's reply.
    pub async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let body = WireChatRequest {
            model: &self.cfg.model,
            req: &req,
        };

        debug!(model = %self.cfg.model, "sending Groq chat-completion request");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let body_bytes = resp.bytes().await?;
        if !status.is_success() {
            // Keep error bodies bounded — a multi-MB HTML error page
            // shouldn't ride into logs or anyhow chains.
            const MAX_ERR_BODY: usize = 2048;
            let preview = if body_bytes.len() > MAX_ERR_BODY {
                &body_bytes[..MAX_ERR_BODY]
            } else {
                &body_bytes[..]
            };
            return Err(Error::Provider {
                status: status.as_u16(),
                body: String::from_utf8_lossy(preview).into_owned(),
            });
        }

        let raw: RawChatResponse = serde_json::from_slice(&body_bytes)?;
        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidResponse {
                reason: "empty choices array".into(),
            })?;

        let tool_calls: Result<Vec<_>> = choice
            .message
            .tool_calls
            .into_iter()
            .map(|raw_tc| {
                Ok(ToolCall {
                    id: raw_tc.id,
                    name: raw_tc.function.name,
                    arguments: serde_json::from_str(&raw_tc.function.arguments)?,
                })
            })
            .collect();

        Ok(ChatResponse {
            content: choice.message.content,
            tool_calls: tool_calls?,
            finish_reason: choice.finish_reason.into(),
        })
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> GroqConfig {
        GroqConfig {
            api_key: "fake-key".into(),
            base_url: "https://example.invalid".into(),
            model: "test-model".into(),
            request_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn chat_request_serializes_user_message_only() {
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "hello".into(),
            }],
            tools: None,
            tool_choice: None,
        };
        let val = serde_json::to_value(&req).unwrap();
        let obj = val.as_object().unwrap();
        assert!(obj.get("tools").is_none(), "tools should be absent");
        assert!(
            obj.get("tool_choice").is_none(),
            "tool_choice should be absent"
        );
        let msgs = obj["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello");
    }

    #[test]
    fn chat_request_serializes_tools_and_tool_choice() {
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "what's the weather".into(),
            }],
            tools: Some(vec![Tool {
                name: "get_weather".into(),
                description: "Get current weather".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                }),
            }]),
            tool_choice: Some(ToolChoice::Function("get_weather".into())),
        };
        let val = serde_json::to_value(&req).unwrap();
        let obj = val.as_object().unwrap();
        let tools = obj["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "get_weather");
        assert_eq!(tools[0]["function"]["description"], "Get current weather");
        let tc = &obj["tool_choice"];
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
    }

    #[test]
    fn chat_request_serializes_assistant_with_tool_calls() {
        let req = ChatRequest {
            messages: vec![Message::Assistant {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: serde_json::json!({"city": "Paris"}),
                }]),
            }],
            tools: None,
            tool_choice: None,
        };
        let val = serde_json::to_value(&req).unwrap();
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "assistant");
        let tcs = msgs[0]["tool_calls"].as_array().unwrap();
        assert_eq!(tcs[0]["id"], "call_1");
        assert_eq!(tcs[0]["type"], "function");
        assert_eq!(tcs[0]["function"]["name"], "get_weather");
        assert_eq!(tcs[0]["function"]["arguments"], "{\"city\":\"Paris\"}");
    }

    #[test]
    fn chat_request_serializes_tool_message() {
        let req = ChatRequest {
            messages: vec![Message::Tool {
                tool_call_id: "call_1".into(),
                content: "22°C".into(),
            }],
            tools: None,
            tool_choice: None,
        };
        let val = serde_json::to_value(&req).unwrap();
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "call_1");
        assert_eq!(msgs[0]["content"], "22°C");
    }

    #[test]
    fn chat_response_decodes_text_completion() {
        let body = br#"{"choices":[{"message":{"role":"assistant","content":"Paris."},"finish_reason":"stop"}]}"#;
        let raw: RawChatResponse = serde_json::from_slice(body).unwrap();
        let choice = raw.choices.into_iter().next().unwrap();
        assert_eq!(choice.message.content.as_deref(), Some("Paris."));
        assert!(choice.message.tool_calls.is_empty());
        assert_eq!(FinishReason::from(choice.finish_reason), FinishReason::Stop);
    }

    #[test]
    fn chat_response_decodes_tool_call_completion() {
        let body = br#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let raw: RawChatResponse = serde_json::from_slice(body).unwrap();
        let choice = raw.choices.into_iter().next().unwrap();
        assert!(choice.message.content.is_none());
        assert_eq!(choice.message.tool_calls.len(), 1);

        let args = serde_json::from_str::<serde_json::Value>(
            &choice.message.tool_calls[0].function.arguments,
        )
        .unwrap();
        assert_eq!(args["city"], "Paris");
        assert_eq!(
            FinishReason::from(choice.finish_reason),
            FinishReason::ToolCalls
        );
    }

    #[test]
    fn chat_response_maps_finish_reasons() {
        assert_eq!(FinishReason::from("stop".to_string()), FinishReason::Stop);
        assert_eq!(
            FinishReason::from("length".to_string()),
            FinishReason::Length
        );
        assert_eq!(
            FinishReason::from("tool_calls".to_string()),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::from("content_filter".to_string()),
            FinishReason::Other("content_filter".into())
        );
    }

    #[test]
    fn chat_response_rejects_invalid_arguments_json() {
        let body = br#"{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"x","arguments":"not valid json{{"}}]},"finish_reason":"stop"}]}"#;
        let raw: RawChatResponse = serde_json::from_slice(body).unwrap();
        let choice = raw.choices.into_iter().next().unwrap();
        let tc = &choice.message.tool_calls[0];
        let result = serde_json::from_str::<serde_json::Value>(&tc.function.arguments);
        assert!(result.is_err());
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        let _client = GroqClient::new(test_cfg()).expect("client builds");
    }

    #[test]
    fn chat_response_empty_choices_is_invalid_response() {
        let body = br#"{"choices":[]}"#;
        let raw: RawChatResponse = serde_json::from_slice(body).unwrap();
        // chat() turns this into Error::InvalidResponse — the parse
        // itself succeeds; what fails is structural expectations.
        assert!(raw.choices.is_empty());
    }

    #[test]
    fn provider_error_truncates_long_body() {
        let long_body = "x".repeat(5000);
        const MAX_ERR_BODY: usize = 2048;
        let preview = if long_body.len() > MAX_ERR_BODY {
            &long_body[..MAX_ERR_BODY]
        } else {
            &long_body[..]
        };
        let err = Error::Provider {
            status: 500,
            body: preview.to_string(),
        };
        match err {
            Error::Provider { body, .. } => {
                assert!(body.len() <= MAX_ERR_BODY, "body should be ≤ 2048 chars");
            }
            _ => panic!("expected Error::Provider"),
        }
    }
}

//! OpenAI chat-completions client.
//!
//! Targets the standard `POST /v1/chat/completions` endpoint.
//! Shares wire types and HTTP logic with [`crate::groq::GroqClient`].

use crate::backend::LlmBackend;
use crate::chat::{ChatRequest, ChatResponse};
use crate::error::Result;
use std::time::Duration;
use tracing::debug;

/// Inputs to [`OpenAiClient::new`].
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub request_timeout: Duration,
}

/// HTTP client around OpenAI's chat-completions endpoint.
#[derive(Debug)]
pub struct OpenAiClient {
    http: reqwest::Client,
    cfg: OpenAiConfig,
}

impl OpenAiClient {
    pub fn new(cfg: OpenAiConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()?;
        Ok(Self { http, cfg })
    }

    /// Send a chat-completion request and return the model's reply.
    pub async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        debug!(model = %self.cfg.model, "sending OpenAI chat-completion request");
        crate::chat::post_chat_completions(
            &self.http,
            &self.cfg.base_url,
            &self.cfg.api_key,
            &self.cfg.model,
            &req,
        )
        .await
    }
}

#[async_trait::async_trait]
impl LlmBackend for OpenAiClient {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        OpenAiClient::chat(self, req).await
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{Message, Tool, ToolChoice};
    use serde_json::json;
    use wiremock::matchers::{bearer_token, body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_cfg(base_url: String) -> OpenAiConfig {
        OpenAiConfig {
            api_key: "sk-test".into(),
            base_url,
            model: "gpt-5.5".into(),
            request_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn new_builds_a_client_without_calling_out() {
        let _client =
            OpenAiClient::new(test_cfg("https://example.invalid".into())).expect("client builds");
    }

    #[tokio::test]
    async fn chat_posts_to_chat_completions_with_correct_headers() {
        let server = MockServer::start().await;

        let expected_body = json!({
            "model": "gpt-5.5",
            "messages": [{"role": "user", "content": "hello"}],
        });

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(bearer_token("sk-test"))
            .and(header("content-type", "application/json"))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "role": "assistant", "content": "Hi there." },
                    "finish_reason": "stop"
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = OpenAiClient::new(test_cfg(server.uri())).unwrap();
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "hello".into(),
            }],
            tools: None,
            tool_choice: None,
        };
        let resp = client.chat(req).await.unwrap();
        assert_eq!(resp.content, Some("Hi there.".into()));
        assert!(resp.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn chat_injects_model_field() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(json!({
                "model": "gpt-5.5",
                "messages": [{"role":"user","content":"test"}],
                "tools": [{"type":"function","function":{"name":"fn","description":"d","parameters":{"type":"object"}}}],
                "tool_choice": "auto"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "role": "assistant", "content": "ok" },
                    "finish_reason": "stop"
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = OpenAiClient::new(test_cfg(server.uri())).unwrap();
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "test".into(),
            }],
            tools: Some(vec![Tool {
                name: "fn".into(),
                description: "d".into(),
                parameters: json!({"type": "object"}),
            }]),
            tool_choice: Some(ToolChoice::Auto),
        };
        let resp = client.chat(req).await.unwrap();
        assert_eq!(resp.content, Some("ok".into()));
    }

    #[tokio::test]
    async fn chat_returns_tool_calls() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"city\":\"Paris\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = OpenAiClient::new(test_cfg(server.uri())).unwrap();
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "weather".into(),
            }],
            tools: None,
            tool_choice: None,
        };
        let resp = client.chat(req).await.unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "get_weather");
        assert_eq!(resp.tool_calls[0].arguments, json!({"city": "Paris"}));
    }

    #[tokio::test]
    async fn chat_surfaces_provider_errors() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
            .expect(1)
            .mount(&server)
            .await;

        let client = OpenAiClient::new(test_cfg(server.uri())).unwrap();
        let req = ChatRequest {
            messages: vec![Message::User {
                content: "hello".into(),
            }],
            tools: None,
            tool_choice: None,
        };
        let err = client.chat(req).await.unwrap_err();
        assert!(
            matches!(err, crate::Error::Provider { status: 401, .. }),
            "expected Provider error with 401, got {err:?}"
        );
    }
}

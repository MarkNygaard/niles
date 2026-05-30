//! Groq chat-completions client.
//!
//! Targets the OpenAI-compatible `POST /openai/v1/chat/completions`
//! endpoint. Wire types live in [`crate::chat`]; this module only
//! holds the Groq-specific client configuration and [`LlmBackend`]
//! implementation.

use crate::backend::LlmBackend;
use crate::chat::{ChatRequest, ChatResponse};
use crate::error::Result;
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

/// HTTP client around Groq's chat-completions endpoint.
#[derive(Debug)]
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
        debug!(model = %self.cfg.model, "sending Groq chat-completion request");
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
impl LlmBackend for GroqClient {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        GroqClient::chat(self, req).await
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
    fn new_builds_a_client_without_calling_out() {
        let _client = GroqClient::new(test_cfg()).expect("client builds");
    }
}

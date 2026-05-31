//! Async trait abstraction over LLM providers.
//!
//! Introduced alongside the second backend (OpenAI) so that
//! `niles-bin` can hold `Arc<dyn LlmBackend>` for Tier 2 dispatch.

use crate::chat::{ChatRequest, ChatResponse};
use crate::error::Result;
use async_trait::async_trait;

/// Provider-agnostic chat-completions backend.
#[async_trait]
pub trait LlmBackend: Send + Sync + std::fmt::Debug {
    /// Send a chat-completion request and return the model's reply.
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>;
}

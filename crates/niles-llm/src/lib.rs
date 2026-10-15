//! Tier 1+2 LLM adapter layer.
//!
//! One client, [`OpenAiClient`], for any OpenAI-compatible
//! chat-completions endpoint: Cerebras for Tier 1 by default, OpenAI
//! for the Tier 2 escalation. Text in, text + tool-call responses out.
//! It sits behind the [`LlmBackend`] trait so `niles-bin` can hold
//! `Arc<dyn LlmBackend>` and tests can use a fake.

mod backend;
mod chat;
pub mod chatgpt;
mod error;
mod openai;

pub use backend::LlmBackend;
pub use chat::{
    ChatRequest, ChatResponse, FinishReason, Message, ReasoningEffort, Tool, ToolCall, ToolChoice,
};
pub use error::{Error, Result};
pub use openai::{OpenAiClient, OpenAiConfig};

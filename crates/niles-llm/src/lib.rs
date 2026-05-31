//! Tier 1+2 LLM adapter layer.
//!
//! v0.1 ships one provider — Groq chat-completions (default model:
//! `openai/gpt-oss-20b` per ARCHITECTURE.md model recommendations) —
//! exposed through [`GroqClient::chat`]. Text in, text + tool-call
//! responses out.
//!
//! v0.2 adds [`OpenAiClient`] as a second backend behind the
//! [`LlmBackend`] trait so `niles-bin` can hold `Arc<dyn LlmBackend>`
//! for Tier 2 escalation.

mod backend;
mod chat;
mod error;
mod groq;
mod openai;

pub use backend::LlmBackend;
pub use chat::{ChatRequest, ChatResponse, FinishReason, Message, Tool, ToolCall, ToolChoice};
pub use error::{Error, Result};
pub use groq::{GroqClient, GroqConfig};
pub use openai::{OpenAiClient, OpenAiConfig};

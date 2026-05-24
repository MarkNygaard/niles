//! Tier 1 LLM adapter layer.
//!
//! v0.1 ships one provider — Groq chat-completions (default model:
//! `openai/gpt-oss-20b` per ARCHITECTURE.md model recommendations) —
//! exposed through [`GroqClient::chat`]. Text in, text + tool-call
//! responses out.
//!
//! No `Llm` trait yet — per repo convention, traits land alongside
//! their second implementation, not the first.

mod error;
mod groq;

pub use error::{Error, Result};
pub use groq::{
    ChatRequest, ChatResponse, FinishReason, GroqClient, GroqConfig, Message, Tool, ToolCall,
    ToolChoice,
};

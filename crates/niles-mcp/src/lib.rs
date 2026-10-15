//! niles-mcp — Niles as an MCP client, for a service that brings its
//! own tools: Linear, and any other server that speaks MCP over
//! Streamable HTTP with a Bearer key.
//!
//! Tools only. The server's resources and prompts are not asked for, and
//! a server's own requests to the client (sampling, elicitation) are not
//! answered: Niles advertises no client capabilities, so a conforming
//! server sends none.

pub mod client;
pub mod error;

pub use client::{CallResult, McpClient, RemoteTool, ServerInfo};
pub use error::{Error, Result};

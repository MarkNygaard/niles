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
pub mod oauth;
pub mod personal;
pub mod public;

pub use client::{Auth, CallResult, McpClient, RemoteTool, ServerInfo};
pub use error::{Error, Result};
pub use oauth::{AuthServer, Pkce, Registration, SignInClient, Tokens, authorize_url};
pub use personal::{
    BrowserSignIn, Credential, Key, MemoryServers, Mode, NewServer, PersonalServer,
    PersonalServers, ServerEdit, SignInKind,
};
pub use public::{Reach, public_url};

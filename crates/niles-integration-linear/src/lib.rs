//! niles-integration-linear — Linear's webhook, which turns a change to
//! an issue into a notification.
//!
//! Linear's tools are not here: they come from Linear's own MCP server,
//! as a tool server (see `niles-mcp`).

pub mod error;
pub mod webhook;

pub use error::{Error, Result};
pub use webhook::{
    WebhookNotification, WebhookPayload, notification_for, parse_webhook, verify_signature,
};

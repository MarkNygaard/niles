//! niles-api — HTTP API surface over the device registry.
//!
//! Provides read and write endpoints for device state, a
//! health check, a live WebSocket event stream, and a Linear webhook.
//!
//! ```text
//! GET  /healthz                 -> 200 "ok"
//! GET  /devices                 -> JSON array of all devices
//! GET  /rooms/{room}            -> JSON array of devices in that room
//! POST /rooms/{room}/{device}   -> 202 Accepted (set light state)
//! GET  /events/stream           -> WebSocket upgrade (live event stream)
//! POST /webhooks/linear         -> 200 OK (Linear webhook)
//! ```

pub mod dto;
pub mod events;
pub mod handlers;
pub mod publish;
pub mod server;
pub mod state;
pub mod webhook;

pub use publish::DevicePublisher;
pub use server::{router, serve};
pub use state::{AppState, LinearWebhookState};

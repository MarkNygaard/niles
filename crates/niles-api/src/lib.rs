//! niles-api — HTTP API surface over the device registry.
//!
//! Read-only for v0.1. Future versions will add write endpoints
//! (set device state, apply scene, etc.) and a WebSocket for live
//! event streams.
//!
//! ```text
//! GET /healthz                 -> 200 "ok"
//! GET /devices                 -> JSON array of all devices
//! GET /rooms/{room}            -> JSON array of devices in that room
//! ```

pub mod dto;
pub mod handlers;
pub mod publish;
pub mod server;
pub mod state;

pub use publish::DevicePublisher;
pub use server::{router, serve};
pub use state::AppState;

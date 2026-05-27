//! niles-api — HTTP API surface over the device registry.
//!
//! Provides read and write endpoints for device state, plus a
//! health check. Future versions will add scene endpoints and a
//! WebSocket for live event streams.
//!
//! ```text
//! GET  /healthz                 -> 200 "ok"
//! GET  /devices                 -> JSON array of all devices
//! GET  /rooms/{room}            -> JSON array of devices in that room
//! POST /rooms/{room}/{device}   -> 202 Accepted (set light state)
//! ```

pub mod dto;
pub mod handlers;
pub mod publish;
pub mod server;
pub mod state;

pub use publish::DevicePublisher;
pub use server::{router, serve};
pub use state::AppState;

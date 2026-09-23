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
//! GET  /config                  -> effective config + overrides + reload info
//! PATCH /config                 -> merge a partial config document
//! DELETE /config/{path}         -> drop one override (dotted path)
//! GET  /config/history          -> recorded changes, oldest first
//! POST /config/undo             -> walk back one change
//! POST /webhooks/linear         -> 200 OK (Linear webhook)
//! ```

pub mod auth;
pub mod captures;
pub mod climate;
pub mod config;
#[cfg(test)]
mod config_tests;
pub mod dto;
pub mod events;
pub mod handlers;
pub mod integrations;
pub mod logs;
pub mod places;
pub mod presence;
pub mod publish;
pub mod scenes;
pub mod secrets;
pub mod server;
pub mod state;
pub mod voices;
#[cfg(feature = "ui")]
pub mod web;
pub mod webhook;

pub use publish::DevicePublisher;
pub use server::{router, serve};
pub use state::{AppState, LinearWebhookState};

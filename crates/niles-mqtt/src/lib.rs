//! niles-mqtt — MQTT client + Zigbee2MQTT device source.
//!
//! Two responsibilities:
//!
//! - [`client`] — a thin async wrapper around `rumqttc` that exposes
//!   `connect` / `subscribe` / `publish` / `next_message` with our
//!   own `Message` type so consumers don't depend on `rumqttc`.
//! - [`z2m`] — Z2M message types (bridge device list, per-device
//!   state) and conversions to `niles_core::Device` / `DeviceState`.
//!
//! Registry wiring (consume Z2M messages, populate
//! `niles_core::DeviceRegistry`, publish events) lands in a follow-up.

pub mod client;
pub mod error;
pub mod source;
pub mod z2m;

pub use client::{DisconnectReason, Message, MqttClient, MqttOptions};
pub use error::{Error, Result};
pub use source::Z2mSource;
pub use z2m::{Z2mDevice, Z2mState};

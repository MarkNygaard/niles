//! niles-mqtt — MQTT client + Zigbee2MQTT / WLED device sources.
//!
//! Three responsibilities:
//!
//! - [`client`] — a thin async wrapper around `rumqttc` that exposes
//!   `connect` / `subscribe` / `publish` / `next_message` with our
//!   own `Message` type so consumers don't depend on `rumqttc`.
//! - [`z2m`] — Z2M message types (bridge device list, per-device state)
//!   and conversions to `niles_core::Device` / `DeviceState`.
//! - [`wled`] — WLED helpers: brightness translation, hex-color parsing,
//!   `/g` `/c` `/status` payload parsers, and `format_wled_command`.
//! - [`command`] — `CommandRouter` that dispatches set commands to the
//!   correct source format (Z2M `/set` vs WLED `/api`).
//!
//! Registry wiring (consume Z2M/WLED messages, populate
//! `niles_core::DeviceRegistry`, publish events) is handled by
//! [`Z2mSource`] and [`WledSource`].

pub mod client;
pub mod command;
pub mod error;
pub mod sink;
pub mod source;
pub mod wled;
pub mod wled_source;
pub mod z2m;

pub use client::{DisconnectReason, Message, MqttClient, MqttOptions, MqttPublisher};
pub use command::CommandRouter;
pub use error::{Error, Result};
pub use sink::{format_set_command, is_actionable};
pub use source::Z2mSource;
pub use wled::format_wled_command;
pub use wled_source::WledSource;
pub use z2m::{Z2mDevice, Z2mState};

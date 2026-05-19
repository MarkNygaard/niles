//! Error types for niles-mqtt.

use thiserror::Error;

/// Errors surfaced by niles-mqtt's public API.
///
/// `rumqttc::ConnectionError` (eventloop disconnects) is intentionally
/// *not* a variant — the pumping task handles those internally and
/// the message channel simply ends. Callers detect that via a `None`
/// from [`crate::MqttClient::next_message`].
#[derive(Debug, Error)]
pub enum Error {
    #[error("MQTT client error: {0}")]
    Client(#[from] rumqttc::ClientError),

    #[error("could not parse Z2M JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("could not derive device from Z2M entry: {0}")]
    Device(#[from] niles_core::Error),

    #[error("invalid Z2M device entry: {reason}")]
    InvalidEntry { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

//! Abstraction over MQTT publishing for device commands.
//!
//! `DevicePublisher` keeps the dependency direction correct:
//! `niles-api` depends on `niles-mqtt`, not the other way around.

use async_trait::async_trait;

#[async_trait]
pub trait DevicePublisher: Send + Sync {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), String>;
}

#[async_trait]
impl DevicePublisher for niles_mqtt::MqttPublisher {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
        niles_mqtt::MqttPublisher::publish(self, &topic, payload)
            .await
            .map_err(|e| e.to_string())
    }
}

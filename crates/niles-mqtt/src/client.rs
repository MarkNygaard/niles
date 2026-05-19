//! MQTT client wrapper.

use crate::error::Result;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions as RmqOptions, QoS};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Configuration for connecting to an MQTT broker.
#[derive(Debug, Clone)]
pub struct MqttOptions {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Keep-alive interval. Defaults to 30s if unset.
    pub keep_alive: Option<Duration>,
}

impl MqttOptions {
    pub fn new(host: impl Into<String>, port: u16, client_id: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            client_id: client_id.into(),
            username: None,
            password: None,
            keep_alive: None,
        }
    }

    pub fn with_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }
}

/// A received MQTT message.
#[derive(Debug, Clone)]
pub struct Message {
    pub topic: String,
    pub payload: Vec<u8>,
}

impl Message {
    /// Borrow the payload as a UTF-8 string. Returns `None` if the
    /// payload isn't valid UTF-8 (Z2M payloads always are, but raw
    /// MQTT can carry arbitrary bytes).
    pub fn payload_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.payload).ok()
    }
}

/// An async MQTT client. Cloning is intentionally not supported —
/// hold one per consumer task and route messages downstream over
/// internal channels.
pub struct MqttClient {
    client: AsyncClient,
    incoming: mpsc::UnboundedReceiver<Message>,
    _event_loop: JoinHandle<()>,
}

impl MqttClient {
    /// Connect to the broker. The eventloop is spawned in a tokio
    /// task that forwards incoming `Publish` packets to an internal
    /// channel; consume them with [`Self::next_message`].
    pub fn connect(opts: MqttOptions) -> Self {
        let mut rmq = RmqOptions::new(opts.client_id, opts.host, opts.port);
        rmq.set_keep_alive(opts.keep_alive.unwrap_or(Duration::from_secs(30)));
        if let (Some(u), Some(p)) = (opts.username, opts.password) {
            rmq.set_credentials(u, p);
        }

        let (client, event_loop) = AsyncClient::new(rmq, 32);
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(pump_events(event_loop, tx));

        Self {
            client,
            incoming: rx,
            _event_loop: handle,
        }
    }

    /// Subscribe to a topic. Supports `+` (single-level) and `#`
    /// (multi-level) wildcards per the MQTT spec.
    pub async fn subscribe(&self, topic: &str) -> Result<()> {
        self.client.subscribe(topic, QoS::AtLeastOnce).await?;
        Ok(())
    }

    /// Publish a payload to a topic.
    pub async fn publish(&self, topic: &str, payload: impl Into<Vec<u8>>) -> Result<()> {
        self.client
            .publish(topic, QoS::AtLeastOnce, false, payload.into())
            .await?;
        Ok(())
    }

    /// Block until the next incoming message arrives, or `None` if
    /// the eventloop has terminated.
    pub async fn next_message(&mut self) -> Option<Message> {
        self.incoming.recv().await
    }
}

/// Background task that pumps the rumqttc `EventLoop` and forwards
/// `Publish` packets to the consumer channel. Exits silently when
/// the consumer drops the receiver or the eventloop errors.
async fn pump_events(mut event_loop: EventLoop, tx: mpsc::UnboundedSender<Message>) {
    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(p))) => {
                let msg = Message {
                    topic: p.topic,
                    payload: p.payload.to_vec(),
                };
                if tx.send(msg).is_err() {
                    // Consumer dropped the receiver — clean shutdown.
                    break;
                }
            }
            Ok(_) => {} // ack/pingresp/etc. — uninteresting here
            Err(e) => {
                // Surface the disconnect so downstream knows the
                // stream ended. rumqttc auto-reconnects on subsequent
                // poll calls, but for v0.1 we exit on first error
                // and let the caller decide how to handle it.
                let _ = e;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_builder_sets_credentials() {
        let opts = MqttOptions::new("h", 1883, "id").with_credentials("u", "p");
        assert_eq!(opts.host, "h");
        assert_eq!(opts.port, 1883);
        assert_eq!(opts.username.as_deref(), Some("u"));
        assert_eq!(opts.password.as_deref(), Some("p"));
    }

    #[test]
    fn message_payload_str_returns_some_for_utf8() {
        let msg = Message {
            topic: "t".into(),
            payload: b"hello".to_vec(),
        };
        assert_eq!(msg.payload_str(), Some("hello"));
    }

    #[test]
    fn message_payload_str_returns_none_for_invalid_utf8() {
        let msg = Message {
            topic: "t".into(),
            payload: vec![0xff, 0xfe, 0xfd],
        };
        assert_eq!(msg.payload_str(), None);
    }
}

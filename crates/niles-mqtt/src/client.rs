//! MQTT client wrapper.

use crate::error::Result;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions as RmqOptions, QoS};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
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
    disconnect: Option<oneshot::Receiver<DisconnectReason>>,
    _event_loop: JoinHandle<()>,
}

/// Why the event-loop task terminated.
#[derive(Debug, Clone)]
pub enum DisconnectReason {
    /// `MqttClient` was dropped while the eventloop was still running.
    /// Normal shutdown from the eventloop's perspective.
    ConsumerDropped,
    /// `rumqttc` returned an error from `poll()`. The string is
    /// `e.to_string()` because rumqttc's errors aren't `Clone`.
    Error(String),
}

impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsumerDropped => f.write_str("consumer dropped the receiver"),
            Self::Error(s) => f.write_str(s),
        }
    }
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
        let (disc_tx, disc_rx) = oneshot::channel();
        let handle = tokio::spawn(pump_events(event_loop, tx, disc_tx));

        Self {
            client,
            incoming: rx,
            disconnect: Some(disc_rx),
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
    /// the eventloop has terminated. On `None`, call
    /// [`Self::last_error`] to retrieve the disconnect reason.
    pub async fn next_message(&mut self) -> Option<Message> {
        self.incoming.recv().await
    }

    /// After [`Self::next_message`] returns `None`, this returns the
    /// reason the eventloop terminated — bad credentials, broker
    /// disconnect, DNS failure, etc. Returns `None` if the eventloop
    /// is still running or the reason has already been consumed.
    ///
    /// Async because it awaits the oneshot from the eventloop task,
    /// which may not have sent yet when `next_message` first returns
    /// `None` (rare race; usually it has).
    pub async fn last_error(&mut self) -> Option<DisconnectReason> {
        let rx = self.disconnect.take()?;
        rx.await.ok()
    }
}

/// Background task that pumps the rumqttc `EventLoop` and forwards
/// `Publish` packets to the consumer channel. Always reports a
/// [`DisconnectReason`] on exit so callers can diagnose failures.
async fn pump_events(
    mut event_loop: EventLoop,
    tx: mpsc::UnboundedSender<Message>,
    disc_tx: oneshot::Sender<DisconnectReason>,
) {
    let reason = loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(p))) => {
                let msg = Message {
                    topic: p.topic,
                    payload: p.payload.to_vec(),
                };
                if tx.send(msg).is_err() {
                    break DisconnectReason::ConsumerDropped;
                }
            }
            Ok(_) => {} // ack/pingresp/etc. — uninteresting here
            Err(e) => {
                // rumqttc auto-reconnects on subsequent poll calls,
                // but for v0.1 we surface the first error and exit.
                // The caller can decide whether to retry.
                break DisconnectReason::Error(e.to_string());
            }
        }
    };
    // If the receiver was dropped (client itself dropped), we don't
    // care — no one's listening.
    let _ = disc_tx.send(reason);
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

    #[test]
    fn disconnect_reason_display() {
        assert_eq!(
            DisconnectReason::ConsumerDropped.to_string(),
            "consumer dropped the receiver"
        );
        assert_eq!(
            DisconnectReason::Error("bad password".into()).to_string(),
            "bad password"
        );
    }

    #[tokio::test]
    async fn last_error_reports_connection_failure() {
        // Connect to a port that isn't bound; pump_events should
        // surface the error through last_error().
        let opts = MqttOptions {
            host: "127.0.0.1".into(),
            port: 1, // privileged port, almost certainly nothing listening
            client_id: "niles-test".into(),
            username: None,
            password: None,
            keep_alive: Some(Duration::from_secs(1)),
        };
        let mut client = MqttClient::connect(opts);
        // next_message returns None when the loop exits with an error
        assert!(client.next_message().await.is_none());
        let reason = client.last_error().await;
        match reason {
            Some(DisconnectReason::Error(s)) => {
                assert!(!s.is_empty(), "expected non-empty error string");
            }
            other => panic!("expected DisconnectReason::Error, got {other:?}"),
        }
    }
}

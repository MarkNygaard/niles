//! MQTT client wrapper.

use crate::error::Result;
use rumqttc::{AsyncClient, ConnAck, Event, EventLoop, Incoming, MqttOptions as RmqOptions, QoS};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

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

/// Why the event-loop task terminated.
///
/// Only emitted on **terminal** exits — once the eventloop has
/// successfully connected (received a `ConnAck`), runtime
/// disconnects are recovered transparently via rumqttc's reconnect
/// machinery; subscriptions are replayed. The eventloop only stops
/// for:
///
/// - the initial connection never succeeding (typically bad
///   credentials, wrong host, or DNS failure), or
/// - the consumer dropping the `MqttClient`.
#[derive(Debug, Clone)]
pub enum DisconnectReason {
    /// `MqttClient` was dropped while the eventloop was still running.
    ConsumerDropped,
    /// `rumqttc` returned an error from `poll()` *before any successful
    /// connection*. The string is `e.to_string()` because rumqttc's
    /// errors aren't `Clone`.
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

/// An async MQTT client with automatic reconnect.
///
/// Once the client has successfully connected once, the eventloop
/// keeps polling forever — on a broker outage rumqttc reconnects
/// transparently and the client re-subscribes to every topic that
/// was passed to [`Self::subscribe`].
///
/// Initial connection failures (bad credentials, unreachable host)
/// still surface via [`Self::last_error`] and terminate the loop,
/// because those almost always require human intervention to fix.
pub struct MqttClient {
    client: AsyncClient,
    incoming: mpsc::UnboundedReceiver<Message>,
    disconnect: Option<oneshot::Receiver<DisconnectReason>>,
    subscriptions: Arc<Mutex<Vec<String>>>,
    event_loop: JoinHandle<()>,
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
        let subscriptions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let handle = tokio::spawn(pump_events(
            event_loop,
            client.clone(),
            tx,
            subscriptions.clone(),
            disc_tx,
        ));

        Self {
            client,
            incoming: rx,
            disconnect: Some(disc_rx),
            subscriptions,
            event_loop: handle,
        }
    }

    /// Subscribe to a topic. Supports `+` (single-level) and `#`
    /// (multi-level) wildcards per the MQTT spec. The topic is
    /// remembered and replayed on every successful reconnect.
    pub async fn subscribe(&self, topic: &str) -> Result<()> {
        // Record before sending so a concurrent reconnect picks it
        // up via the replay path even if the immediate send below
        // races against an in-progress reconnect.
        {
            let mut subs = self.subscriptions.lock().unwrap();
            if !subs.iter().any(|t| t == topic) {
                subs.push(topic.to_string());
            }
        }
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
    /// the eventloop has terminated (either initial-connect failure
    /// or `MqttClient` drop). On `None`, call [`Self::last_error`] to
    /// retrieve the reason.
    pub async fn next_message(&mut self) -> Option<Message> {
        self.incoming.recv().await
    }

    /// After [`Self::next_message`] returns `None`, this returns the
    /// reason the eventloop terminated. Consumed on first read.
    ///
    /// Async because it awaits the oneshot from the eventloop task,
    /// which may not have sent yet when `next_message` first returns
    /// `None` (rare race; usually it has).
    pub async fn last_error(&mut self) -> Option<DisconnectReason> {
        let rx = self.disconnect.take()?;
        rx.await.ok()
    }

    /// Topics currently tracked for reconnect replay. Order matches
    /// the original `subscribe` calls; duplicates are deduped at
    /// subscription time.
    pub fn subscriptions(&self) -> Vec<String> {
        self.subscriptions.lock().unwrap().clone()
    }
}

impl Drop for MqttClient {
    fn drop(&mut self) {
        // Abort the eventloop so the task doesn't keep running with
        // its (sleeping) backoff timer after the consumer is gone.
        self.event_loop.abort();
    }
}

const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Background task that pumps the rumqttc `EventLoop`. After the
/// first successful `ConnAck`, this runs forever — runtime errors
/// are logged and retried with exponential backoff (capped). Initial
/// connection failures are reported once via the oneshot and end the
/// task so the caller can diagnose them.
async fn pump_events(
    mut event_loop: EventLoop,
    client: AsyncClient,
    tx: mpsc::UnboundedSender<Message>,
    subscriptions: Arc<Mutex<Vec<String>>>,
    disc_tx: oneshot::Sender<DisconnectReason>,
) {
    let mut ever_connected = false;
    let mut backoff = RECONNECT_INITIAL_BACKOFF;

    let reason = loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::ConnAck(ConnAck { .. }))) => {
                if ever_connected {
                    info!("MQTT reconnected; replaying subscriptions");
                } else {
                    info!("MQTT connected");
                }
                ever_connected = true;
                backoff = RECONNECT_INITIAL_BACKOFF;

                let to_replay: Vec<String> = subscriptions.lock().unwrap().clone();
                for topic in to_replay {
                    match client.subscribe(&topic, QoS::AtLeastOnce).await {
                        Ok(()) => debug!("resubscribed to {topic}"),
                        Err(e) => warn!("failed to resubscribe to {topic}: {e}"),
                    }
                }
            }
            Ok(Event::Incoming(Incoming::Publish(p))) => {
                let msg = Message {
                    topic: p.topic,
                    payload: p.payload.to_vec(),
                };
                if tx.send(msg).is_err() {
                    break DisconnectReason::ConsumerDropped;
                }
            }
            Ok(_) => {} // ack/pingresp/etc.
            Err(e) => {
                if !ever_connected {
                    // The first error before we've ever connected is
                    // almost always a misconfiguration (bad creds,
                    // wrong host, DNS). Surface it and exit — the
                    // caller likely wants to bail rather than spin
                    // forever on `BadUserNamePassword`.
                    break DisconnectReason::Error(e.to_string());
                }
                warn!("MQTT disconnected: {e}; reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX_BACKOFF);
            }
        }
    };
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
    async fn last_error_reports_initial_connection_failure() {
        // Connect to a port that isn't bound. Because no ConnAck ever
        // arrives, the first poll error is fatal and surfaces via
        // last_error().
        let opts = MqttOptions {
            host: "127.0.0.1".into(),
            port: 1,
            client_id: "niles-test".into(),
            username: None,
            password: None,
            keep_alive: Some(Duration::from_secs(1)),
        };
        let mut client = MqttClient::connect(opts);
        assert!(client.next_message().await.is_none());
        match client.last_error().await {
            Some(DisconnectReason::Error(s)) => {
                assert!(!s.is_empty(), "expected non-empty error string");
            }
            other => panic!("expected DisconnectReason::Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_records_topic_for_reconnect_replay() {
        // Build a client against an unreachable broker. We don't care
        // whether the subscribe send succeeds — only that the topic is
        // recorded so reconnect replay would pick it up.
        let opts = MqttOptions::new("127.0.0.1", 1, "niles-test");
        let client = MqttClient::connect(opts);

        // Bounded timeout: the subscribe may hang briefly while
        // rumqttc decides the broker is gone; we just need the record
        // to land. Use a short timeout and ignore the result.
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            client.subscribe("zigbee2mqtt/bridge/devices"),
        )
        .await;

        assert_eq!(
            client.subscriptions(),
            vec!["zigbee2mqtt/bridge/devices".to_string()]
        );
    }

    #[tokio::test]
    async fn subscribe_dedupes_repeated_topics() {
        let opts = MqttOptions::new("127.0.0.1", 1, "niles-test");
        let client = MqttClient::connect(opts);
        let _ = tokio::time::timeout(Duration::from_millis(500), client.subscribe("a/b")).await;
        let _ = tokio::time::timeout(Duration::from_millis(500), client.subscribe("a/b")).await;
        assert_eq!(client.subscriptions(), vec!["a/b".to_string()]);
    }
}

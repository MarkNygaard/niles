//! MQTT client wrapper.

use crate::error::Result;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions as RmqOptions, QoS};
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
///
/// During a runtime disconnect the eventloop sleeps on a backoff
/// timer rather than draining outgoing requests, so concurrent
/// [`Self::subscribe`] / [`Self::publish`] calls buffer up to the
/// internal channel's capacity (32) and then block until reconnect.
/// Treat them as best-effort while the broker is unreachable.
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
        // rumqttc defaults to a 10 KB packet limit, which is far too
        // small for Z2M's `bridge/devices` payload (26 KB for one
        // bulb in a real Z2M; 100+ KB at a typical home). Without
        // this, every reconnect triggers Z2M to re-publish the
        // retained inventory, which exceeds the limit, which forces
        // another disconnect — an infinite reconnect loop. 1 MB
        // gives ample headroom for both directions.
        rmq.set_max_packet_size(1024 * 1024, 256 * 1024);
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
            let mut subs = self.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Return a clone-able publish-only handle backed by this
    /// client's connection. Useful when one part of the program
    /// owns the `MqttClient` (e.g. `Z2mSource` consuming messages)
    /// and other tasks need to publish on the same connection
    /// without opening a second one.
    pub fn publisher(&self) -> MqttPublisher {
        MqttPublisher {
            client: self.client.clone(),
        }
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
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for MqttClient {
    fn drop(&mut self) {
        // Abort the eventloop so the task doesn't keep running with
        // its (sleeping) backoff timer after the consumer is gone.
        self.event_loop.abort();
    }
}

/// Clone-able publish-only handle. Backed by the same `AsyncClient`
/// (and therefore the same eventloop + connection) as the
/// `MqttClient` it was obtained from. Publishes go through that
/// shared eventloop, so they survive runtime disconnects and benefit
/// from the same backoff/reconnect behavior.
///
/// Dropping a `MqttPublisher` does not affect the underlying
/// connection — that lives and dies with the `MqttClient`. If the
/// `MqttClient` is dropped, subsequent `publish` calls will return
/// an error because the eventloop task has been aborted.
#[derive(Clone)]
pub struct MqttPublisher {
    client: AsyncClient,
}

impl MqttPublisher {
    pub async fn publish(&self, topic: &str, payload: impl Into<Vec<u8>>) -> Result<()> {
        self.client
            .publish(topic, QoS::AtLeastOnce, false, payload.into())
            .await?;
        Ok(())
    }
}

const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// A subscription replay running beside the eventloop, cancelled when
/// this is dropped.
///
/// The replay cannot run inline: every `subscribe` waits for room in
/// rumqttc's bounded request channel, and the only thing that empties
/// that channel is the task polling the eventloop. Inline, the 33rd
/// topic waited on its own task forever. Out here it waits on the
/// eventloop instead, which keeps the backpressure and loses the
/// deadlock.
struct Replay(JoinHandle<()>);

impl Replay {
    fn start(client: AsyncClient, topics: Vec<String>) -> Self {
        Self(tokio::spawn(async move {
            for topic in topics {
                match client.subscribe(&topic, QoS::AtLeastOnce).await {
                    Ok(()) => debug!("resubscribed to {topic}"),
                    Err(e) => warn!("failed to resubscribe to {topic}: {e}"),
                }
            }
        }))
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        self.0.abort();
    }
}

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
    // At most one replay, and only for the connection it was started
    // for. A replay outliving its connection would keep feeding the
    // channel while the next one starts its own, and the broker would
    // see the overlap as a second round of subscriptions.
    let mut replay: Option<Replay> = None;

    let reason = loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                if ever_connected {
                    info!("MQTT reconnected; replaying subscriptions");
                    // Replay only on RECONNECT. On the initial connect,
                    // the caller's subscribe() calls already queued the
                    // subscriptions through rumqttc — replaying here
                    // would double-subscribe, which Mosquitto handles
                    // poorly when multiple overlapping patterns are in
                    // play (silently wedges message delivery).
                    let to_replay: Vec<String> = subscriptions
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    replay.replace(Replay::start(client.clone(), to_replay));
                } else {
                    info!("MQTT connected");
                }
                ever_connected = true;
                backoff = RECONNECT_INITIAL_BACKOFF;
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
                // rumqttc has just moved whatever was queued into its
                // pending list, which a clean session discards on
                // reconnect. Stopping the replay now keeps it from
                // refilling the channel during the backoff with
                // subscriptions meant for a connection that is gone.
                replay.take();
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

    // The reconnect tests below grew out of an outside review, which
    // found the replay deadlocking on its own request channel once more
    // than 32 topics were remembered (its capacity). 32 is the control,
    // 33 the first count that hung, and 200 is some 66 WLED strips at
    // three topics each.

    /// Read one MQTT packet: the fixed-header byte and the body.
    async fn read_packet(socket: &mut tokio::net::TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let header = socket.read_u8().await?;
        let mut len = 0usize;
        let mut multiplier = 1usize;
        loop {
            let byte = socket.read_u8().await?;
            len += usize::from(byte & 127) * multiplier;
            if byte & 128 == 0 {
                break;
            }
            multiplier *= 128;
        }
        let mut body = vec![0u8; len];
        socket.read_exact(&mut body).await?;
        Ok((header, body))
    }

    /// Take a connection's CONNECT and accept it.
    async fn accept_session(listener: &tokio::net::TcpListener) -> tokio::net::TcpStream {
        use tokio::io::AsyncWriteExt;
        let (mut socket, _) = listener.accept().await.unwrap();
        let (header, _) = read_packet(&mut socket).await.unwrap();
        assert_eq!(header, 0x10, "expected CONNECT");
        socket.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        socket
    }

    /// Read SUBSCRIBE packets, acknowledging each, until `want`
    /// distinct topics have arrived, and return every topic in the
    /// order it came — repeats included, so a caller can tell a replay
    /// that ran twice. Fails on anything other than a SUBSCRIBE: nothing
    /// else should be on the wire.
    async fn take_subscriptions(socket: &mut tokio::net::TcpStream, want: usize) -> Vec<String> {
        use tokio::io::AsyncWriteExt;
        let mut topics = Vec::new();
        let mut distinct = std::collections::BTreeSet::new();
        while distinct.len() < want {
            let (header, body) = read_packet(socket).await.unwrap();
            assert_eq!(header, 0x82, "expected SUBSCRIBE, got {header:#x}");
            let mut at = 2;
            let mut granted = 0u8;
            while at < body.len() {
                let len = usize::from(u16::from_be_bytes([body[at], body[at + 1]]));
                let topic = std::str::from_utf8(&body[at + 2..at + 2 + len]).unwrap();
                topics.push(topic.to_string());
                distinct.insert(topic.to_string());
                at += 2 + len + 1;
                granted += 1;
            }
            let mut suback = vec![0x90, 2 + granted, body[0], body[1]];
            suback.extend(std::iter::repeat_n(0x01, usize::from(granted)));
            socket.write_all(&suback).await.unwrap();
        }
        topics
    }

    fn remembered(count: usize) -> (Arc<Mutex<Vec<String>>>, Vec<String>) {
        let topics: Vec<String> = (0..count).map(|i| format!("replay/{i}")).collect();
        (Arc::new(Mutex::new(topics.clone())), topics)
    }

    /// Every remembered topic, each exactly once.
    fn assert_replayed_once(mut received: Vec<String>, mut expected: Vec<String>) {
        received.sort();
        expected.sort();
        assert_eq!(received, expected);
    }

    /// Connect once, drop the connection, and check the reconnect
    /// subscribes to every remembered topic.
    async fn reconnect_replays_every_subscription(count: usize) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let broker = tokio::spawn(async move {
            drop(accept_session(&listener).await);
            let mut socket = accept_session(&listener).await;
            take_subscriptions(&mut socket, count).await
        });
        let opts = RmqOptions::new("niles-test-replay", addr.ip().to_string(), addr.port());
        let (client, event_loop) = AsyncClient::new(opts, 32);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (disc_tx, _disc_rx) = oneshot::channel();
        let (subscriptions, expected) = remembered(count);
        let pump = tokio::spawn(pump_events(event_loop, client, tx, subscriptions, disc_tx));
        let result = tokio::time::timeout(Duration::from_secs(10), broker).await;
        pump.abort();
        let _ = pump.await;
        let received = result
            .unwrap_or_else(|_| panic!("reconnect replay of {count} topics stalled"))
            .unwrap();
        assert_replayed_once(received, expected);
    }

    #[tokio::test]
    async fn reconnect_replays_subscriptions_that_fill_the_request_queue() {
        reconnect_replays_every_subscription(32).await;
    }

    #[tokio::test]
    async fn reconnect_replays_more_subscriptions_than_the_request_queue_holds() {
        reconnect_replays_every_subscription(33).await;
    }

    #[tokio::test]
    async fn reconnect_replays_hundreds_of_subscriptions() {
        reconnect_replays_every_subscription(200).await;
    }

    #[tokio::test]
    async fn a_reconnect_in_the_middle_of_a_replay_starts_it_again() {
        // The second connection closes as soon as it is accepted, with
        // the replay for it barely started. The third has to get every
        // topic once: not the leftovers of the second's replay, and not
        // those leftovers followed by a fresh round.
        let count = 100;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let broker = tokio::spawn(async move {
            drop(accept_session(&listener).await);
            drop(accept_session(&listener).await);
            let mut socket = accept_session(&listener).await;
            take_subscriptions(&mut socket, count).await
        });
        let opts = RmqOptions::new("niles-test-replay", addr.ip().to_string(), addr.port());
        let (client, event_loop) = AsyncClient::new(opts, 32);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (disc_tx, _disc_rx) = oneshot::channel();
        let (subscriptions, expected) = remembered(count);
        let pump = tokio::spawn(pump_events(event_loop, client, tx, subscriptions, disc_tx));
        let result = tokio::time::timeout(Duration::from_secs(15), broker).await;
        pump.abort();
        let _ = pump.await;
        let received = result.expect("replay stalled after the reconnect").unwrap();
        assert_replayed_once(received, expected);
    }

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

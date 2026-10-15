//! MQTT for plugins: the connections Niles holds on a plugin's behalf.
//!
//! `mqtt.connect` starts a task here and hands the instance a
//! [`Connection`] at once, so no host call ever waits on the network.
//! The task resolves the broker, checks every address with the same
//! rules as HTTP ([`MqttTarget::allows`]), connects to the address that
//! passed, does TLS as the manifest says, speaks MQTT 3.1.1 with
//! rumqttc's packet codec and session state, pings, and reconnects with
//! a backoff, subscribing again each time. What arrives goes into the
//! plugin's [`Inbox`], delivered by its thread one call at a time.
//!
//! `trust-device` is trust on first use: the first time a connection to
//! a broker succeeds, the SHA-256 of its certificate's public key is
//! kept in the plugin's storage under a key the plugin cannot reach. A
//! different key on a later connection stops the connection before
//! anything is sent, and says so with `certificate-changed`; the plugin
//! asks the household and calls `accept-certificate`. Pinning the key
//! rather than the certificate lets a device renew an expiring
//! certificate on the same key without a word.
//!
//! rumqttc's own event loop is not used because it resolves the host
//! itself: the connection must go to the address that was checked.

use crate::bindings::mqtt as wit;
use crate::bindings::niles::integration::types::HostError;
use crate::log::Level;
use crate::manifest::TlsMode;
use crate::policy::MqttTarget;
use crate::runtime::{House, Plugin};
use futures_util::{SinkExt, StreamExt};
use rumqttc::{
    Codec, ConnectReturnCode, Login, MqttState, Packet, PingReq, QoS, Request, StateError,
    Subscribe, SubscribeFilter, SubscribeReasonCode, Unsubscribe,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

/// Connections per plugin.
pub const MAX_CONNECTIONS: usize = 4;
/// Subscriptions per connection.
pub const MAX_SUBSCRIPTIONS: usize = 64;
/// The largest message delivered or published. A Bambu Lab printer's
/// full report is some tens of KiB; four times the largest seen is room.
pub const MAX_PAYLOAD: usize = 256 * 1024;
/// The largest packet read at all. Between this and [`MAX_PAYLOAD`] a
/// message is dropped and counted; past it the connection is dropped,
/// since the codec cannot skip it.
const MAX_PACKET: usize = 1024 * 1024;
/// The longest topic or filter.
pub const MAX_TOPIC: usize = 1024;
/// Messages waiting for a plugin, and their bytes; past either the
/// oldest go.
pub const INBOX: usize = 256;
pub const INBOX_BYTES: usize = 8 * 1024 * 1024;
/// Publishes per second per connection, and the burst allowed.
pub const PUBLISH_RATE: f64 = 20.0;
pub const PUBLISH_BURST: f64 = 50.0;
/// The waits before each reconnect; the last repeats.
const BACKOFF: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];
/// Connected this long, a drop starts the backoff from the beginning.
const STEADY: Duration = Duration::from_secs(60);
/// For the TCP connection, TLS and the broker's answer, each; and for
/// any one write.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
/// QoS 1 and 2 messages in flight at once.
const INFLIGHT: u16 = 100;
/// Keys in the plugin's storage that are Niles's, not the plugin's.
pub const RESERVED: &str = ".niles/";
/// Notes about dropped messages, at most this often.
const NOTE_EVERY: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// What `connect` was asked for, checked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Options {
    pub client_id: String,
    pub login: Option<(String, String)>,
    pub keep_alive: u16,
    pub clean_session: bool,
}

impl Options {
    pub fn from_wit(plugin: &str, o: wit::ConnectOptions) -> Result<Self, String> {
        let client_id = match o.client_id {
            Some(id) => {
                if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_graphic()) {
                    return Err(format!(
                        "client-id {id:?} is 1 to 64 printable characters, no spaces"
                    ));
                }
                id
            }
            None => format!("niles-{plugin}-{:08x}", rand::random::<u32>()),
        };
        let keep_alive = o.keep_alive.unwrap_or(60);
        if !(5..=1200).contains(&keep_alive) {
            return Err("keep-alive is 5 to 1200 seconds".into());
        }
        let long = |s: &Option<String>| s.as_ref().is_some_and(|s| s.len() > 1024);
        if long(&o.username) || long(&o.password) {
            return Err("a username or password is at most 1024 bytes".into());
        }
        let login = match (o.username, o.password) {
            (Some(user), password) => Some((user, password.unwrap_or_default())),
            (None, Some(_)) => {
                return Err("MQTT 3.1.1 has no password without a username".into());
            }
            (None, None) => None,
        };
        Ok(Self {
            client_id,
            login,
            keep_alive,
            clean_session: o.clean_session.unwrap_or(true),
        })
    }
}

fn qos(q: wit::Qos) -> QoS {
    match q {
        wit::Qos::AtMostOnce => QoS::AtMostOnce,
        wit::Qos::AtLeastOnce => QoS::AtLeastOnce,
        wit::Qos::ExactlyOnce => QoS::ExactlyOnce,
    }
}

/// A topic a plugin publishes to: no wildcards, nothing empty.
fn check_topic(topic: &str) -> Result<(), HostError> {
    if topic.is_empty() || topic.len() > MAX_TOPIC || topic.contains(['+', '#', '\0']) {
        return Err(HostError::Invalid(format!(
            "{topic:?} is not a topic: 1 to {MAX_TOPIC} bytes, without + or #"
        )));
    }
    Ok(())
}

/// A filter a plugin subscribes to: `+` a whole level, `#` the last.
fn check_filter(filter: &str) -> Result<(), HostError> {
    if filter.len() > MAX_TOPIC || filter.contains('\0') || !rumqttc::valid_filter(filter) {
        return Err(HostError::Invalid(format!(
            "{filter:?} is not a filter: levels split by /, + for a whole level, # only at the end"
        )));
    }
    Ok(())
}

/// Whether `topic` is one `filter` covers.
fn matches(topic: &str, filter: &str) -> bool {
    topic == filter || rumqttc::matches(topic, filter)
}

/// One subscription, as the plugin asked for it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Subscription {
    qos: QoS,
    latest_only: bool,
}

/// What a connection's task and its handle both see.
#[derive(Default)]
struct Shared {
    connected: AtomicBool,
    closed: AtomicBool,
    subscriptions: Mutex<BTreeMap<String, Subscription>>,
    bucket: Mutex<Option<Bucket>>,
    /// The key a `trust-device` broker presented in its last
    /// `certificate-changed`, until it is accepted.
    changed: Mutex<Option<String>>,
}

/// The publish rate, as a token bucket.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(now: Instant) -> Self {
        Self {
            tokens: PUBLISH_BURST,
            at: now,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        let refill = now.saturating_duration_since(self.at).as_secs_f64() * PUBLISH_RATE;
        self.tokens = (self.tokens + refill).min(PUBLISH_BURST);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

enum Command {
    Subscribe(String, QoS),
    Unsubscribe(String),
    Publish(rumqttc::Publish),
    Accept(String),
    Close,
}

/// An instance's handle on one connection. Dropping it closes the
/// connection, which is how an instance that goes takes its connections
/// with it.
pub(crate) struct Connection {
    id: u32,
    broker: String,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
    house: Arc<House>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Close);
    }
}

impl Connection {
    pub fn subscribe(
        &self,
        filter: String,
        q: wit::Qos,
        latest_only: bool,
    ) -> Result<(), HostError> {
        check_filter(&filter)?;
        {
            let mut subscriptions = lock(&self.shared.subscriptions);
            if !subscriptions.contains_key(&filter) && subscriptions.len() >= MAX_SUBSCRIPTIONS {
                return Err(HostError::Limit(format!(
                    "at most {MAX_SUBSCRIPTIONS} subscriptions per connection"
                )));
            }
            subscriptions.insert(
                filter.clone(),
                Subscription {
                    qos: qos(q),
                    latest_only,
                },
            );
        }
        self.trace(|| Trace::Subscribe {
            conn: self.id,
            filter: filter.clone(),
        });
        let _ = self.commands.send(Command::Subscribe(filter, qos(q)));
        Ok(())
    }

    pub fn unsubscribe(&self, filter: String) -> Result<(), HostError> {
        check_filter(&filter)?;
        if lock(&self.shared.subscriptions).remove(&filter).is_some() {
            self.trace(|| Trace::Unsubscribe {
                conn: self.id,
                filter: filter.clone(),
            });
            let _ = self.commands.send(Command::Unsubscribe(filter));
        }
        Ok(())
    }

    pub fn publish(
        &self,
        topic: String,
        payload: Vec<u8>,
        q: wit::Qos,
        retain: bool,
    ) -> Result<(), HostError> {
        check_topic(&topic)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(HostError::Limit(format!(
                "a message is at most {} KiB",
                MAX_PAYLOAD / 1024
            )));
        }
        if !self.shared.connected.load(Ordering::SeqCst) {
            return Err(HostError::Unavailable(format!(
                "not connected to {} right now",
                self.broker
            )));
        }
        let now = Instant::now();
        let allowed = lock(&self.shared.bucket)
            .get_or_insert_with(|| Bucket::full(now))
            .take(now);
        if !allowed {
            return Err(HostError::Limit(format!(
                "at most {PUBLISH_RATE} messages a second, in bursts of {PUBLISH_BURST}"
            )));
        }
        self.trace(|| Trace::Publish {
            conn: self.id,
            topic: topic.clone(),
            payload: payload.clone(),
        });
        let mut publish = rumqttc::Publish::new(topic, qos(q), payload);
        publish.retain = retain;
        let _ = self.commands.send(Command::Publish(publish));
        Ok(())
    }

    pub fn accept_certificate(&self) -> Result<(), HostError> {
        let presented = lock(&self.shared.changed).take().ok_or_else(|| {
            HostError::Invalid(format!(
                "{}'s certificate has not changed; there is nothing to accept",
                self.broker
            ))
        })?;
        let _ = self.commands.send(Command::Accept(presented));
        Ok(())
    }

    /// Tell the watch, if there is one; `trace` is only made then.
    fn trace(&self, trace: impl FnOnce() -> Trace) {
        if let Some(watch) = &self.house.mqtt_watch {
            watch(&trace());
        }
    }
}

/// Start a connection's task, and hand back its handle.
pub(crate) fn open(
    plugin: Arc<Plugin>,
    house: Arc<House>,
    id: u32,
    target: MqttTarget,
    options: Options,
) -> Connection {
    let (commands, receiver) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared::default());
    let broker = target.broker.clone();
    let task = Task {
        id,
        plugin,
        house: house.clone(),
        target,
        options,
        shared: shared.clone(),
        commands: receiver,
        too_large: 0,
        noted_too_large: None,
    };
    house.handle.spawn(task.run());
    Connection {
        id,
        broker,
        shared,
        commands,
        house,
    }
}

/// Something a connection hands the plugin.
#[derive(Debug, Clone)]
pub(crate) enum Delivery {
    Message {
        conn: u32,
        topic: String,
        payload: Vec<u8>,
        latest_only: bool,
    },
    Status {
        conn: u32,
        status: wit::ConnectionStatus,
    },
}

impl Delivery {
    pub fn conn(&self) -> u32 {
        match self {
            Delivery::Message { conn, .. } | Delivery::Status { conn, .. } => *conn,
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Delivery::Message { topic, payload, .. } => topic.len() + payload.len(),
            Delivery::Status { .. } => 0,
        }
    }
}

/// What waits for a plugin: at most [`INBOX`] messages and
/// [`INBOX_BYTES`] of them, the oldest dropped first. A message on a
/// `latest-only` subscription replaces one still waiting on the same
/// topic, where it was. A status is never dropped.
#[derive(Debug, Default)]
pub(crate) struct Inbox {
    items: VecDeque<Delivery>,
    bytes: usize,
    dropped: u64,
}

impl Inbox {
    pub fn push(&mut self, delivery: Delivery) {
        if let Delivery::Message {
            conn,
            topic,
            latest_only: true,
            ..
        } = &delivery
        {
            let waiting = self.items.iter_mut().find(|d| {
                matches!(d, Delivery::Message { conn: c, topic: t, latest_only: true, .. }
                    if c == conn && t == topic)
            });
            if let Some(waiting) = waiting {
                self.bytes = self.bytes - waiting.bytes() + delivery.bytes();
                *waiting = delivery;
                return;
            }
        }
        self.bytes += delivery.bytes();
        self.items.push_back(delivery);
        while self.items.len() > INBOX || self.bytes > INBOX_BYTES {
            let Some(oldest) = self
                .items
                .iter()
                .position(|d| matches!(d, Delivery::Message { .. }))
            else {
                break;
            };
            if let Some(gone) = self.items.remove(oldest) {
                self.bytes -= gone.bytes();
                self.dropped += 1;
            }
        }
    }

    pub fn pop(&mut self) -> Option<Delivery> {
        let delivery = self.items.pop_front()?;
        self.bytes -= delivery.bytes();
        Some(delivery)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
    }

    /// Messages dropped since this was last asked.
    pub fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }
}

/// What happens on a plugin's connections, for `niles plugin dev`.
#[derive(Debug, Clone, PartialEq)]
pub enum Trace {
    Connecting {
        conn: u32,
        broker: String,
        address: SocketAddr,
    },
    Status {
        conn: u32,
        broker: String,
        status: String,
    },
    Subscribe {
        conn: u32,
        filter: String,
    },
    Unsubscribe {
        conn: u32,
        filter: String,
    },
    Publish {
        conn: u32,
        topic: String,
        payload: Vec<u8>,
    },
    Message {
        conn: u32,
        topic: String,
        payload: Vec<u8>,
    },
}

impl std::fmt::Display for Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trace::Connecting {
                conn,
                broker,
                address,
            } => write!(f, "mqtt {conn} {broker}: connecting to {address}"),
            Trace::Status {
                conn,
                broker,
                status,
            } => write!(f, "mqtt {conn} {broker}: {status}"),
            Trace::Subscribe { conn, filter } => write!(f, "mqtt {conn} + {filter}"),
            Trace::Unsubscribe { conn, filter } => write!(f, "mqtt {conn} - {filter}"),
            Trace::Publish {
                conn,
                topic,
                payload,
            } => write!(f, "mqtt {conn} > {topic} {}", abbreviate(payload)),
            Trace::Message {
                conn,
                topic,
                payload,
            } => write!(f, "mqtt {conn} < {topic} {}", abbreviate(payload)),
        }
    }
}

/// A payload as one short line: its start as text, and its size.
pub fn abbreviate(payload: &[u8]) -> String {
    const SHOWN: usize = 80;
    let text = String::from_utf8_lossy(payload);
    let mut line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(SHOWN)
        .collect();
    if text.chars().count() > SHOWN {
        line.push('…');
    }
    let size = match payload.len() {
        n if n < 1024 => format!("{n} B"),
        n => format!("{:.1} KiB", n as f64 / 1024.0),
    };
    format!("{line} ({size})")
}

/// A status as the plugin's log says it.
pub fn describe(status: &wit::ConnectionStatus) -> String {
    match status {
        wit::ConnectionStatus::Connected => "connected".into(),
        wit::ConnectionStatus::Disconnected(why) => format!("disconnected: {why}"),
        wit::ConnectionStatus::Failed(why) => format!("stopped trying: {why}"),
        wit::ConnectionStatus::CertificateChanged(change) => format!(
            "its certificate changed (trusted {}, now {}); nothing was sent, and it waits to be accepted",
            change.trusted, change.presented
        ),
    }
}

/// Where a broker's pinned key is kept in the plugin's storage.
pub(crate) fn pin_key(target: &MqttTarget) -> String {
    let address = Sha256::digest(format!("{}:{}", target.host, target.port));
    let short: String = address[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{RESERVED}mqtt-pin/{}/{short}", target.broker)
}

/// The SHA-256 of a certificate's public key, as hex pairs — what a
/// trusted device's key is pinned as. Read whatever the certificate's
/// version: see `niles_tls`.
pub use niles_tls::fingerprint;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// TLS checked against the public certificate authorities.
fn verifying() -> rustls::ClientConfig {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// TLS that takes the device's own certificate if its key is the one
/// pinned, or any key when none is pinned yet. Handshake signatures are
/// still checked, so whoever answers holds that key.
fn trusting(pinned: Option<String>, seen: Arc<Mutex<Option<String>>>) -> rustls::ClientConfig {
    let provider = provider();
    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustDevice {
            provider,
            pinned,
            seen,
        }))
        .with_no_client_auth()
}

#[derive(Debug)]
struct TrustDevice {
    provider: Arc<rustls::crypto::CryptoProvider>,
    pinned: Option<String>,
    /// The key the device presented, for the caller to pin or report.
    seen: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for TrustDevice {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = fingerprint(end_entity).map_err(rustls::Error::General)?;
        *lock(&self.seen) = Some(presented.clone());
        match &self.pinned {
            Some(pinned) if *pinned != presented => Err(rustls::Error::General(
                "the certificate is not the one trusted before".into(),
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        niles_tls::signed_by_key(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
            niles_tls::Tls::V12,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        niles_tls::signed_by_key(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
            niles_tls::Tls::V13,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}

type Wire = Framed<Box<dyn Stream>, Codec>;

/// How a session ended.
enum End {
    /// The plugin closed it, or its instance went.
    Closed,
    /// Worth trying again. `connected_for` is how long it was up, if it was.
    Lost {
        reason: String,
        connected_for: Option<Duration>,
    },
    /// Not worth trying again until something changes.
    Failed(String),
    /// A `trust-device` broker's key is not the pinned one.
    Changed { trusted: String, presented: String },
}

fn lost(reason: impl Into<String>) -> End {
    End::Lost {
        reason: reason.into(),
        connected_for: None,
    }
}

/// Why sending or receiving failed: as the plugin and its card are
/// told it, and the library's own words, for the log at debug level.
struct Why {
    said: String,
    raw: Option<String>,
}

impl Why {
    fn from_error(context: &str, e: &(dyn std::error::Error + 'static)) -> Self {
        Why {
            said: format!("{context}: {}", crate::human::said(e, "the broker")),
            raw: Some(e.to_string()),
        }
    }
}

impl From<String> for Why {
    fn from(said: String) -> Self {
        Why { said, raw: None }
    }
}

impl From<&str> for Why {
    fn from(said: &str) -> Self {
        said.to_string().into()
    }
}

/// One connection's task.
struct Task {
    id: u32,
    plugin: Arc<Plugin>,
    house: Arc<House>,
    target: MqttTarget,
    options: Options,
    shared: Arc<Shared>,
    commands: mpsc::UnboundedReceiver<Command>,
    too_large: u64,
    noted_too_large: Option<Instant>,
}

impl Task {
    async fn run(mut self) {
        let mut attempt = 0;
        loop {
            let end = self.session().await;
            self.shared.connected.store(false, Ordering::SeqCst);
            match end {
                End::Closed => return,
                End::Lost {
                    reason,
                    connected_for,
                } => {
                    if connected_for.is_some_and(|up| up >= STEADY) {
                        attempt = 0;
                    }
                    let wait = BACKOFF[attempt.min(BACKOFF.len() - 1)];
                    attempt += 1;
                    self.status(
                        wit::ConnectionStatus::Disconnected(reason),
                        &format!("; trying again in {} s", wait.as_secs()),
                    );
                    if !self.idle(Some(wait)).await {
                        return;
                    }
                }
                End::Failed(reason) => {
                    self.status(wit::ConnectionStatus::Failed(reason), "");
                    self.idle(None).await;
                    return;
                }
                End::Changed { trusted, presented } => {
                    *lock(&self.shared.changed) = Some(presented.clone());
                    self.status(
                        wit::ConnectionStatus::CertificateChanged(wit::CertificateChange {
                            trusted,
                            presented,
                        }),
                        "",
                    );
                    let Some(accepted) = self.accepted().await else {
                        return;
                    };
                    self.pin(&accepted).await;
                    self.plugin.note(
                        Level::Warn,
                        &format!(
                            "MQTT {}: the new certificate was accepted ({accepted})",
                            self.target.broker
                        ),
                    );
                    attempt = 0;
                }
            }
        }
    }

    /// Wait `wait`, or until closed when none. False if it was closed.
    async fn idle(&mut self, wait: Option<Duration>) -> bool {
        let sleep = tokio::time::sleep(wait.unwrap_or(Duration::from_secs(3600)));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep, if wait.is_some() => return true,
                command = self.commands.recv() => match command {
                    None | Some(Command::Close) => return false,
                    Some(_) => {}
                },
            }
        }
    }

    /// Wait for `accept-certificate`: the key accepted, or none if closed.
    async fn accepted(&mut self) -> Option<String> {
        loop {
            match self.commands.recv().await? {
                Command::Accept(key) => return Some(key),
                Command::Close => return None,
                _ => {}
            }
        }
    }

    fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    /// Tell the watch, if there is one; `trace` is only made then.
    fn trace(&self, trace: impl FnOnce() -> Trace) {
        if let Some(watch) = &self.house.mqtt_watch {
            watch(&trace());
        }
    }

    fn deliver(&self, delivery: Delivery) {
        if !self.closed() {
            self.plugin.queue.push_mqtt(delivery);
        }
    }

    /// Tell the plugin and its log; `detail` is for the log only.
    fn status(&self, status: wit::ConnectionStatus, detail: &str) {
        if self.closed() {
            return;
        }
        let line = describe(&status);
        let level = match status {
            wit::ConnectionStatus::Connected => Level::Info,
            wit::ConnectionStatus::Disconnected(_) => Level::Warn,
            _ => Level::Error,
        };
        self.plugin.note(
            level,
            &format!("MQTT {}: {line}{detail}", self.target.broker),
        );
        self.trace(|| Trace::Status {
            conn: self.id,
            broker: self.target.broker.clone(),
            status: line,
        });
        self.deliver(Delivery::Status {
            conn: self.id,
            status,
        });
    }

    /// What the plugin is told; the library's words go to its log, at
    /// debug level, for whoever needs them.
    fn explained(&self, why: Why) -> String {
        if let Some(raw) = why.raw {
            self.plugin
                .note(Level::Debug, &format!("MQTT {}: {raw}", self.target.broker));
        }
        why.said
    }

    async fn pinned(&self) -> Option<String> {
        let key = pin_key(&self.target);
        match self.house.store.get(&self.plugin.id, &key).await {
            Ok(pinned) => pinned,
            Err(e) => {
                tracing::warn!(plugin = %self.plugin.id, "could not read a pinned key: {e}");
                None
            }
        }
    }

    async fn pin(&self, key: &str) {
        let at = pin_key(&self.target);
        match self.house.store.set(&self.plugin.id, &at, key).await {
            Ok(crate::store::Stored::Yes) => {}
            Ok(crate::store::Stored::OverQuota) => self.plugin.note(
                Level::Warn,
                &format!(
                    "MQTT {}: storage is full, so the certificate is not pinned",
                    self.target.broker
                ),
            ),
            Err(e) => self.plugin.note(
                Level::Warn,
                &format!(
                    "MQTT {}: the certificate could not be pinned: {e}",
                    self.target.broker
                ),
            ),
        }
    }

    /// The address to connect to: the literal one, or the first the
    /// name resolves to that the rules allow.
    async fn address(&self) -> Result<IpAddr, End> {
        if let Some(ip) = self.target.literal() {
            return self.target.allows(ip).map(|()| ip).map_err(End::Failed);
        }
        let host = &self.target.host;
        let lookup = tokio::net::lookup_host((host.as_str(), self.target.port));
        let addresses: Vec<IpAddr> = match tokio::time::timeout(NETWORK_TIMEOUT, lookup).await {
            Err(_) => return Err(lost(format!("{host} could not be looked up in time"))),
            Ok(Err(e)) => {
                let why = Why::from_error(&format!("{host} could not be found"), &e);
                return Err(lost(self.explained(why)));
            }
            Ok(Ok(found)) => found.map(|a| a.ip()).collect(),
        };
        let mut refusal = None;
        for ip in addresses {
            match self.target.allows(ip) {
                Ok(()) => return Ok(ip),
                Err(why) => {
                    refusal.get_or_insert(why);
                }
            }
        }
        Err(End::Failed(
            refusal.unwrap_or_else(|| format!("{host} has no address")),
        ))
    }

    async fn session(&mut self) -> End {
        let ip = match self.address().await {
            Ok(ip) => ip,
            Err(end) => return end,
        };
        let address = SocketAddr::new(ip, self.target.port);
        self.trace(|| Trace::Connecting {
            conn: self.id,
            broker: self.target.broker.clone(),
            address,
        });
        let tcp =
            match tokio::time::timeout(NETWORK_TIMEOUT, tokio::net::TcpStream::connect(address))
                .await
            {
                Err(_) => return lost(format!("{address} did not answer")),
                Ok(Err(e)) => {
                    let why = Why::from_error(&format!("could not connect to {address}"), &e);
                    return lost(self.explained(why));
                }
                Ok(Ok(tcp)) => tcp,
            };
        let _ = tcp.set_nodelay(true);
        let (stream, presented) = match self.tls(tcp).await {
            Ok(secured) => secured,
            Err(end) => return end,
        };
        if self.closed() {
            return End::Closed;
        }
        let mut wire: Wire = Framed::new(
            stream,
            Codec {
                max_incoming_size: MAX_PACKET,
                max_outgoing_size: MAX_PACKET,
            },
        );
        if let Err(end) = self.handshake(&mut wire).await {
            return end;
        }
        if let Some(key) = presented {
            self.pin(&key).await;
            self.plugin.note(
                Level::Info,
                &format!(
                    "MQTT {}: trusting the certificate it presented ({key})",
                    self.target.broker
                ),
            );
        }
        self.connected(wire, address).await
    }

    /// TLS as the manifest says. For a `trust-device` broker seen for the
    /// first time, the key to pin once the broker has accepted us.
    async fn tls(
        &self,
        tcp: tokio::net::TcpStream,
    ) -> Result<(Box<dyn Stream>, Option<String>), End> {
        let host = &self.target.host;
        let name = ServerName::try_from(host.clone())
            .map_err(|_| End::Failed(format!("{host} is not a name TLS can check")))?;
        let (config, pinned, seen) = match self.target.tls {
            TlsMode::Plain => return Ok((Box::new(tcp), None)),
            TlsMode::Verify => (verifying(), None, None),
            TlsMode::TrustDevice => {
                let pinned = self.pinned().await;
                let seen = Arc::new(Mutex::new(None));
                (trusting(pinned.clone(), seen.clone()), pinned, Some(seen))
            }
        };
        let connector = rumqttc::tokio_rustls::TlsConnector::from(Arc::new(config));
        let handshake = tokio::time::timeout(NETWORK_TIMEOUT, connector.connect(name, tcp)).await;
        let presented = seen.as_ref().and_then(|seen| lock(seen).clone());
        match handshake {
            Err(_) => Err(lost(format!("{host} did not finish TLS in time"))),
            Ok(Ok(tls)) => Ok((Box::new(tls), presented.filter(|_| pinned.is_none()))),
            Ok(Err(e)) => {
                if let (Some(trusted), Some(presented)) = (pinned, presented)
                    && trusted != presented
                {
                    return Err(End::Changed { trusted, presented });
                }
                let certificate = e
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                    .is_some_and(|e| matches!(e, rustls::Error::InvalidCertificate(_)));
                let why = self.explained(Why::from_error(&format!("TLS with {host}"), &e));
                if certificate {
                    Err(End::Failed(why))
                } else {
                    Err(lost(why))
                }
            }
        }
    }

    /// CONNECT, and the broker's answer.
    async fn handshake(&self, wire: &mut Wire) -> Result<(), End> {
        let mut connect = rumqttc::Connect::new(self.options.client_id.clone());
        connect.keep_alive = self.options.keep_alive;
        connect.clean_session = self.options.clean_session;
        connect.login = self
            .options
            .login
            .as_ref()
            .map(|(user, password)| Login::new(user.clone(), password.clone()));
        send(wire, Packet::Connect(connect))
            .await
            .map_err(|why| lost(self.explained(why)))?;
        let answer = tokio::time::timeout(NETWORK_TIMEOUT, wire.next())
            .await
            .map_err(|_| lost("the broker did not answer the connection in time"))?;
        let ack = match answer {
            Some(Ok(Packet::ConnAck(ack))) => ack,
            Some(Ok(other)) => return Err(lost(format!("the broker answered {other:?}"))),
            Some(Err(e)) => {
                let why = Why::from_error("the broker's answer", &e);
                return Err(lost(self.explained(why)));
            }
            // Some brokers say nothing to a wrong password: they hang up.
            None => {
                return Err(lost(
                    "the broker closed the connection before accepting it; \
                     some brokers do that for a wrong username or password",
                ));
            }
        };
        match ack.code {
            ConnectReturnCode::Success => Ok(()),
            ConnectReturnCode::ServiceUnavailable => Err(lost("the broker is unavailable")),
            ConnectReturnCode::BadUserNamePassword => {
                Err(End::Failed("wrong username or password".into()))
            }
            ConnectReturnCode::NotAuthorized => Err(End::Failed(
                "not authorised: wrong username or password, or no access".into(),
            )),
            ConnectReturnCode::BadClientId => {
                Err(End::Failed("the broker refused the client id".into()))
            }
            ConnectReturnCode::RefusedProtocolVersion => {
                Err(End::Failed("the broker does not speak MQTT 3.1.1".into()))
            }
        }
    }

    async fn connected(&mut self, mut wire: Wire, address: SocketAddr) -> End {
        let since = Instant::now();
        let up = |reason: String| End::Lost {
            reason,
            connected_for: Some(since.elapsed()),
        };
        let mut state = MqttState::new(INFLIGHT, false);
        let mut asked: HashMap<u16, Vec<String>> = HashMap::new();
        let again: Vec<SubscribeFilter> = lock(&self.shared.subscriptions)
            .iter()
            .map(|(filter, s)| SubscribeFilter::new(filter.clone(), s.qos))
            .collect();
        if !again.is_empty() {
            let request = Request::Subscribe(Subscribe::new_many(again));
            if let Err(why) = outgoing(&mut state, &mut wire, request, &mut asked).await {
                return up(self.explained(why));
            }
        }
        self.shared.connected.store(true, Ordering::SeqCst);
        self.status(
            wit::ConnectionStatus::Connected,
            &format!(" to {} ({address})", self.target.host),
        );
        let keep_alive = Duration::from_secs(u64::from(self.options.keep_alive));
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + keep_alive, keep_alive);
        loop {
            tokio::select! {
                incoming = wire.next() => match incoming {
                    Some(Ok(packet)) => {
                        if let Err(why) = self.incoming(&mut state, &mut wire, packet, &mut asked).await {
                            return up(self.explained(why));
                        }
                    }
                    Some(Err(rumqttc::mqttbytes::Error::PayloadSizeLimitExceeded(n))) => {
                        return up(format!(
                            "the broker sent a message of {} KiB; at most {} KiB",
                            n / 1024,
                            MAX_PACKET / 1024
                        ));
                    }
                    Some(Err(e)) => {
                        let why = Why::from_error("the broker sent something unreadable", &e);
                        return up(self.explained(why));
                    }
                    None => return up("the broker closed the connection".into()),
                },
                command = self.commands.recv() => {
                    let request = match command {
                        None | Some(Command::Close) => {
                            let _ = send(&mut wire, Packet::Disconnect).await;
                            return End::Closed;
                        }
                        Some(Command::Accept(_)) => continue,
                        Some(Command::Subscribe(filter, qos)) => {
                            Request::Subscribe(Subscribe::new(filter, qos))
                        }
                        Some(Command::Unsubscribe(filter)) => {
                            Request::Unsubscribe(Unsubscribe::new(filter))
                        }
                        Some(Command::Publish(publish)) => Request::Publish(publish),
                    };
                    if let Err(why) = outgoing(&mut state, &mut wire, request, &mut asked).await {
                        return up(self.explained(why));
                    }
                }
                _ = ping.tick() => {
                    let request = Request::PingReq(PingReq);
                    if let Err(why) = outgoing(&mut state, &mut wire, request, &mut asked).await {
                        return up(self.explained(why));
                    }
                }
            }
        }
    }

    async fn incoming(
        &mut self,
        state: &mut MqttState,
        wire: &mut Wire,
        packet: Packet,
        asked: &mut HashMap<u16, Vec<String>>,
    ) -> Result<(), Why> {
        match &packet {
            Packet::Publish(publish) => self.received(&publish.topic, &publish.payload),
            Packet::SubAck(ack) => {
                let filters = asked.remove(&ack.pkid).unwrap_or_default();
                let refused: Vec<&str> = ack
                    .return_codes
                    .iter()
                    .zip(
                        filters
                            .iter()
                            .map(String::as_str)
                            .chain(std::iter::repeat("?")),
                    )
                    .filter(|(code, _)| matches!(code, SubscribeReasonCode::Failure))
                    .map(|(_, filter)| filter)
                    .collect();
                if !refused.is_empty() {
                    self.plugin.note(
                        Level::Warn,
                        &format!(
                            "MQTT {}: the broker refused the subscription to {}",
                            self.target.broker,
                            refused.join(", ")
                        ),
                    );
                }
            }
            _ => {}
        }
        let reply = state
            .handle_incoming_packet(packet)
            .map_err(|e| describe_state(&e));
        state.events.clear();
        if let Some(reply) = reply? {
            send(wire, reply).await?;
        }
        Ok(())
    }

    fn received(&mut self, topic: &str, payload: &[u8]) {
        if payload.len() > MAX_PAYLOAD {
            self.too_large += 1;
            if self
                .noted_too_large
                .is_none_or(|at| at.elapsed() >= NOTE_EVERY)
            {
                self.plugin.note(
                    Level::Warn,
                    &format!(
                        "MQTT {}: {} message(s) over {} KiB were dropped, the last on {topic}",
                        self.target.broker,
                        self.too_large,
                        MAX_PAYLOAD / 1024
                    ),
                );
                self.too_large = 0;
                self.noted_too_large = Some(Instant::now());
            }
            return;
        }
        let latest_only = {
            let subscriptions = lock(&self.shared.subscriptions);
            let mut matching = subscriptions
                .iter()
                .filter(|(filter, _)| matches(topic, filter))
                .map(|(_, s)| s.latest_only)
                .peekable();
            if matching.peek().is_none() {
                return;
            }
            matching.all(|latest| latest)
        };
        self.trace(|| Trace::Message {
            conn: self.id,
            topic: topic.to_string(),
            payload: payload.to_vec(),
        });
        self.deliver(Delivery::Message {
            conn: self.id,
            topic: topic.to_string(),
            payload: payload.to_vec(),
            latest_only,
        });
    }
}

/// Hand `request` to the session state and send what it says to.
async fn outgoing(
    state: &mut MqttState,
    wire: &mut Wire,
    request: Request,
    asked: &mut HashMap<u16, Vec<String>>,
) -> Result<(), Why> {
    let filters: Option<Vec<String>> = match &request {
        Request::Subscribe(s) => Some(s.filters.iter().map(|f| f.path.clone()).collect()),
        _ => None,
    };
    let packet = state
        .handle_outgoing_packet(request)
        .map_err(|e| describe_state(&e))?;
    let Some(packet) = packet else {
        return Ok(());
    };
    if let (Packet::Subscribe(s), Some(filters)) = (&packet, filters) {
        asked.insert(s.pkid, filters);
    }
    send(wire, packet).await
}

async fn send(wire: &mut Wire, packet: Packet) -> Result<(), Why> {
    match tokio::time::timeout(NETWORK_TIMEOUT, wire.send(packet)).await {
        Err(_) => Err("the broker stopped taking what Niles sends".into()),
        Ok(Err(e)) => Err(Why::from_error("could not send to the broker", &e)),
        Ok(Ok(())) => Ok(()),
    }
}

fn describe_state(e: &StateError) -> String {
    match e {
        StateError::AwaitPingResp => "the broker stopped answering pings".into(),
        StateError::ConnectionAborted => "the broker closed the connection".into(),
        other => format!("the session broke: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(topic: &str, payload: &[u8], latest_only: bool) -> Delivery {
        Delivery::Message {
            conn: 1,
            topic: topic.into(),
            payload: payload.to_vec(),
            latest_only,
        }
    }

    fn payload(d: &Delivery) -> &[u8] {
        match d {
            Delivery::Message { payload, .. } => payload,
            Delivery::Status { .. } => b"",
        }
    }

    #[test]
    fn a_full_inbox_drops_the_oldest_messages_and_counts_them() {
        let mut inbox = Inbox::default();
        inbox.push(Delivery::Status {
            conn: 1,
            status: wit::ConnectionStatus::Connected,
        });
        for i in 0..INBOX + 3 {
            inbox.push(message("a/b", i.to_string().as_bytes(), false));
        }
        assert_eq!(
            inbox.take_dropped(),
            4,
            "the status is kept, so one more goes"
        );
        assert_eq!(inbox.take_dropped(), 0);
        assert!(matches!(inbox.pop(), Some(Delivery::Status { .. })));
        assert_eq!(payload(&inbox.pop().unwrap()), b"4");
    }

    #[test]
    fn the_inbox_holds_eight_mebibytes_at_most() {
        let mut inbox = Inbox::default();
        let big = vec![0u8; MAX_PAYLOAD];
        for _ in 0..40 {
            inbox.push(message("cam/frame", &big, false));
        }
        assert_eq!(
            inbox.take_dropped(),
            40 - (INBOX_BYTES / (MAX_PAYLOAD + 9)) as u64
        );
        assert!(inbox.bytes <= INBOX_BYTES);
    }

    #[test]
    fn latest_only_keeps_the_newest_per_topic_in_its_place() {
        let mut inbox = Inbox::default();
        inbox.push(message("power", b"1", true));
        inbox.push(message("report", b"partial 1", false));
        inbox.push(message("power", b"2", true));
        inbox.push(message("report", b"partial 2", false));
        let order: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.pop())
            .map(|d| payload(&d).to_vec())
            .collect();
        assert_eq!(
            order,
            vec![b"2".to_vec(), b"partial 1".to_vec(), b"partial 2".to_vec()],
            "partial reports are all kept"
        );
        assert_eq!(inbox.bytes, 0);
    }

    #[test]
    fn the_publish_rate_allows_a_burst_then_twenty_a_second() {
        let start = Instant::now();
        let mut bucket = Bucket::full(start);
        let burst = (0..100).filter(|_| bucket.take(start)).count();
        assert_eq!(burst, PUBLISH_BURST as usize);
        assert!(!bucket.take(start));
        let later = start + Duration::from_millis(500);
        let refilled = (0..100).filter(|_| bucket.take(later)).count();
        assert_eq!(refilled, 10);
    }

    #[test]
    fn topics_and_filters_are_checked() {
        assert!(check_topic("device/01S00A/request").is_ok());
        assert!(check_topic("").is_err());
        assert!(check_topic("a/+/b").is_err());
        assert!(check_topic("a/#").is_err());
        assert!(check_topic(&"x".repeat(MAX_TOPIC + 1)).is_err());
        assert!(check_filter("device/+/report").is_ok());
        assert!(check_filter("tele/#").is_ok());
        assert!(check_filter("a/#/b").is_err());
        assert!(check_filter("a/b+").is_err());
        assert!(check_filter("").is_err());
        assert!(matches("device/01S00A/report", "device/+/report"));
        assert!(matches("$SYS/broker/uptime", "$SYS/broker/uptime"));
        assert!(!matches("device/01S00A/request", "device/+/report"));
    }

    fn options(o: wit::ConnectOptions) -> Result<Options, String> {
        Options::from_wit("printer", o)
    }

    fn none() -> wit::ConnectOptions {
        wit::ConnectOptions {
            client_id: None,
            username: None,
            password: None,
            keep_alive: None,
            clean_session: None,
        }
    }

    #[test]
    fn connect_options_have_defaults_and_limits() {
        let o = options(none()).unwrap();
        assert!(o.client_id.starts_with("niles-printer-"), "{}", o.client_id);
        assert_eq!(o.client_id.len(), "niles-printer-".len() + 8);
        assert_eq!(o.keep_alive, 60);
        assert!(o.clean_session);
        assert_eq!(o.login, None);

        let o = options(wit::ConnectOptions {
            username: Some("bblp".into()),
            password: Some("12345678".into()),
            ..none()
        })
        .unwrap();
        assert_eq!(o.login, Some(("bblp".into(), "12345678".into())));

        for bad in [
            wit::ConnectOptions {
                password: Some("x".into()),
                ..none()
            },
            wit::ConnectOptions {
                keep_alive: Some(0),
                ..none()
            },
            wit::ConnectOptions {
                client_id: Some("has space".into()),
                ..none()
            },
            wit::ConnectOptions {
                client_id: Some("x".repeat(65)),
                ..none()
            },
        ] {
            assert!(options(bad).is_err());
        }
    }

    #[test]
    fn payloads_print_short() {
        assert_eq!(abbreviate(b"{\"a\":1}"), "{\"a\":1} (7 B)");
        let long = format!("{{\"print\":\"{}\"}}", "x".repeat(2000));
        let line = abbreviate(long.as_bytes());
        assert!(line.contains('…') && line.ends_with("(2.0 KiB)"), "{line}");
        assert!(line.chars().count() < 100);
    }

    #[test]
    fn a_pin_is_kept_under_niles_own_prefix() {
        let policy = crate::policy::MqttPolicy::new(
            &crate::manifest::Permissions {
                mqtt: vec![crate::manifest::MqttBroker {
                    name: "printer".into(),
                    host: "192.168.1.40".into(),
                    port: None,
                    tls: TlsMode::TrustDevice,
                }],
                ..Default::default()
            },
            |_| None,
        );
        let target = policy.target("printer").unwrap().unwrap();
        let key = pin_key(&target);
        assert!(key.starts_with(".niles/mqtt-pin/printer/"), "{key}");
        assert!(key.len() <= crate::store::MAX_KEY);
    }
}

#[cfg(test)]
mod real_device {
    use super::*;

    /// A TLS handshake with a real device, trusting it as `trust-device`
    /// does, without logging in.
    ///
    /// `NILES_TEST_TLS_DEVICE=192.168.1.40:8883 cargo test -p niles-plugins -- --ignored real_device`
    #[tokio::test]
    #[ignore = "needs NILES_TEST_TLS_DEVICE"]
    async fn a_real_device_shakes_hands() {
        let Ok(address) = std::env::var("NILES_TEST_TLS_DEVICE") else {
            return;
        };
        let (host, _) = address.rsplit_once(':').expect("host:port");
        let seen = Arc::new(Mutex::new(None));
        let connector =
            rumqttc::tokio_rustls::TlsConnector::from(Arc::new(trusting(None, seen.clone())));
        let tcp = tokio::net::TcpStream::connect(&address)
            .await
            .expect("reachable");
        let name = ServerName::try_from(host.to_string()).expect("a name");
        connector.connect(name, tcp).await.expect("the handshake");
        assert!(lock(&seen).is_some(), "a key was seen");
        println!("key: {}", lock(&seen).clone().unwrap());
    }
}

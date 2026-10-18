//! Host-owned, permission-scoped WebSocket connections for plugins.
#[cfg(test)]
#[path = "websocket_inbox_tests.rs"]
mod inbox_tests;
#[cfg(test)]
#[path = "websocket_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
use crate::bindings::niles::integration::types::HostError;
use crate::bindings::v0_4::niles::integration::websocket as wit;
use crate::runtime::{House, Plugin};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use url::Url;

pub const MAX_CONNECTIONS: usize = 4;
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const INBOX_COUNT: usize = 128;
pub const INBOX_BYTES: usize = 4 * 1024 * 1024;
pub const OUTBOX_COUNT: usize = 32;
pub const OUTBOX_BYTES: usize = 1024 * 1024;

const BACKOFF: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

#[derive(Debug, Clone)]
pub(crate) enum Delivery {
    Message {
        conn: u32,
        generation: u64,
        message: wit::Message,
    },
    Status {
        conn: u32,
        generation: u64,
        status: wit::ConnectionStatus,
    },
}
impl Delivery {
    pub fn conn(&self) -> u32 {
        match self {
            Self::Message { conn, .. } | Self::Status { conn, .. } => *conn,
        }
    }
    pub fn generation(&self) -> u64 {
        match self {
            Self::Message { generation, .. } | Self::Status { generation, .. } => *generation,
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Message { message, .. } => match message {
                wit::Message::Text(s) => s.len(),
                wit::Message::Binary(b) => b.len(),
            },
            _ => 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct Inbox {
    items: VecDeque<Delivery>,
    bytes: usize,
    messages: usize,
    statuses: usize,
    overflow: VecDeque<(u32, u64, u32)>,
    streams: std::collections::BTreeMap<u32, u64>,
}
impl Inbox {
    pub fn register(&mut self, conn: u32) {
        if self.streams.len() < MAX_CONNECTIONS {
            self.streams.insert(conn, 1);
        }
    }
    pub fn remove(&mut self, conn: u32) {
        self.streams.remove(&conn);
        self.purge(conn);
        self.overflow.retain(|(c, _, _)| *c != conn);
    }
    fn purge(&mut self, conn: u32) {
        let mut retained = VecDeque::with_capacity(self.items.len());
        let mut lost: BTreeMap<u64, u32> = BTreeMap::new();
        for item in self.items.drain(..) {
            if item.conn() == conn {
                if matches!(item, Delivery::Message { .. }) {
                    *lost.entry(item.generation()).or_default() += 1;
                    self.messages -= 1;
                    self.bytes -= item.bytes();
                } else {
                    self.statuses -= 1;
                }
            } else {
                retained.push_back(item);
            }
        }
        self.items = retained;
        if let Some((generation, count)) = lost.into_iter().next() {
            self.note_overflow(conn, generation, count);
        }
    }
    fn note_overflow(&mut self, conn: u32, generation: u64, count: u32) {
        if count == 0 {
            return;
        }
        if let Some((_, queued_generation, total)) =
            self.overflow.iter_mut().find(|(c, _, _)| *c == conn)
        {
            *total = total.saturating_add(count);
            *queued_generation = generation;
        } else {
            self.overflow.retain(|(c, _, _)| *c != conn);
            if self.overflow.len() >= MAX_CONNECTIONS {
                self.overflow.pop_front();
            }
            self.overflow.push_back((conn, generation, count));
        }
    }
    fn advance(&mut self, conn: u32, generation: u64) {
        let mut retained = VecDeque::with_capacity(self.items.len());
        let mut lost = 0u32;
        for item in self.items.drain(..) {
            if item.conn() == conn && item.generation() < generation {
                if matches!(item, Delivery::Message { .. }) {
                    lost = lost.saturating_add(1);
                    self.messages -= 1;
                    self.bytes -= item.bytes();
                } else if matches!(
                    &item,
                    Delivery::Status {
                        status: wit::ConnectionStatus::Disconnected(_)
                            | wit::ConnectionStatus::Failed(_),
                        ..
                    }
                ) {
                    retained.push_back(item);
                } else {
                    self.statuses -= 1;
                }
            } else {
                retained.push_back(item);
            }
        }
        self.items = retained;
        if lost > 0 {
            self.note_overflow(conn, generation, lost);
        }
    }
    pub fn push(&mut self, d: Delivery) {
        let Some(&generation) = self.streams.get(&d.conn()) else {
            return;
        };
        if d.generation() < generation {
            return;
        }
        if d.generation() > generation {
            self.advance(d.conn(), d.generation());
            self.streams.insert(d.conn(), d.generation());
        }
        if matches!(d, Delivery::Message { .. }) {
            self.bytes += d.bytes();
            self.messages += 1;
            self.items.push_back(d);
            while self.messages > INBOX_COUNT || self.bytes > INBOX_BYTES {
                if let Some(i) = self
                    .items
                    .iter()
                    .position(|i| matches!(i, Delivery::Message { .. }))
                {
                    if let Some(v) = self.items.remove(i) {
                        self.bytes -= v.bytes();
                        self.messages -= 1;
                        let (conn, generation) = (v.conn(), v.generation());
                        self.note_overflow(conn, generation, 1);
                    }
                } else {
                    break;
                }
            }
        } else {
            let key = (d.conn(), d.generation());
            if matches!(
                d,
                Delivery::Status {
                    status: wit::ConnectionStatus::Connected,
                    ..
                }
            ) && self.items.iter().any(|i| matches!(i, Delivery::Status { conn, generation, status: wit::ConnectionStatus::Connected, .. } if (*conn, *generation) == key)) {
                return;
            }
            if matches!(&d, Delivery::Status { status: wit::ConnectionStatus::Disconnected(_) | wit::ConnectionStatus::Failed(_), .. })
                && let Some(pos) = self.items.iter().position(|item| matches!(item, Delivery::Status { conn, status: wit::ConnectionStatus::Disconnected(_) | wit::ConnectionStatus::Failed(_), .. } if *conn == d.conn())) {
                    self.items.remove(pos);
                    self.statuses -= 1;
            }
            self.statuses += 1;
            self.items.push_back(d);
        }
    }
    pub fn pop(&mut self) -> Option<Delivery> {
        if let Some((conn, generation, count)) = self.overflow.pop_front() {
            return Some(Delivery::Status {
                conn,
                generation,
                status: wit::ConnectionStatus::Overflow(count),
            });
        }
        let d = match self.items.pop_front() {
            Some(item) => item,
            None => {
                let (conn, generation, count) = self.overflow.pop_front()?;
                return Some(Delivery::Status {
                    conn,
                    generation,
                    status: wit::ConnectionStatus::Overflow(count),
                });
            }
        };
        if let Delivery::Status { .. } = d {
            self.statuses -= 1;
        }
        if let Delivery::Message { .. } = d {
            self.messages -= 1;
        }
        self.bytes -= d.bytes();
        Some(d)
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
        self.messages = 0;
        self.statuses = 0;
        self.overflow.clear();
        self.streams.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.overflow.is_empty()
    }
}

enum Command {
    Send { message: Message, generation: u64 },
    Close,
}
pub(crate) struct Shared {
    connected: AtomicBool,
    closed: AtomicBool,
    pub generation: AtomicU64,
    out_count: AtomicUsize,
    out_bytes: AtomicUsize,
}
pub(crate) struct Connection {
    owner: Option<(Arc<Plugin>, u32)>,
    pub shared: Arc<Shared>,
    tx: mpsc::Sender<Command>,
    task: tokio::task::AbortHandle,
    handle: tokio::runtime::Handle,
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some((plugin, id)) = &self.owner {
            plugin.queue.remove_websocket(*id);
        }
        let explicitly_closed = self.shared.closed.swap(true, Ordering::SeqCst);
        self.shared.connected.store(false, Ordering::SeqCst);
        if !explicitly_closed {
            self.task.abort();
        }
    }
}
impl Connection {
    pub fn send(&self, message: wit::Message) -> Result<(), HostError> {
        let size = match &message {
            wit::Message::Text(s) => s.len(),
            wit::Message::Binary(b) => b.len(),
        };
        if size > MAX_MESSAGE {
            return Err(HostError::Limit(format!(
                "a WebSocket message is at most {} KiB",
                MAX_MESSAGE / 1024
            )));
        }
        if self.shared.closed.load(Ordering::SeqCst)
            || !self.shared.connected.load(Ordering::SeqCst)
        {
            return Err(HostError::Unavailable(
                "the WebSocket is not connected".into(),
            ));
        }
        let count = self.shared.out_count.fetch_add(1, Ordering::SeqCst);
        let bytes = self.shared.out_bytes.fetch_add(size, Ordering::SeqCst);
        if count >= OUTBOX_COUNT || bytes + size > OUTBOX_BYTES {
            self.shared.out_count.fetch_sub(1, Ordering::SeqCst);
            self.shared.out_bytes.fetch_sub(size, Ordering::SeqCst);
            return Err(HostError::Limit("the WebSocket send queue is full".into()));
        }
        let frame = match message {
            wit::Message::Text(s) => Message::Text(s.into()),
            wit::Message::Binary(b) => Message::Binary(b.into()),
        };
        let generation = self.shared.generation.load(Ordering::SeqCst);
        self.tx
            .try_send(Command::Send {
                message: frame,
                generation,
            })
            .map_err(|_| {
                self.shared.out_count.fetch_sub(1, Ordering::SeqCst);
                self.shared.out_bytes.fetch_sub(size, Ordering::SeqCst);
                HostError::Unavailable("the WebSocket send queue is unavailable".into())
            })
    }
    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.connected.store(false, Ordering::SeqCst);
        if self.tx.try_send(Command::Close).is_err() {
            self.task.abort();
        } else {
            let task = self.task.clone();
            self.handle.spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                task.abort();
            });
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WsTarget {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub reach: Reach,
    pub loopback: bool,
}
impl WsTarget {
    pub fn allows(&self, ip: IpAddr) -> Result<(), String> {
        match self.reach {
            Reach::Home => crate::policy::ws_address_allowed(ip, true, self.loopback),
            Reach::Internet => crate::policy::ws_address_allowed(ip, false, self.loopback),
            Reach::Either => crate::policy::ws_address_allowed(ip, true, self.loopback)
                .or_else(|_| crate::policy::ws_address_allowed(ip, false, self.loopback)),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    Home,
    Internet,
    Either,
}
#[derive(Clone)]
pub(crate) struct WsPolicy {
    grants: Vec<(String, Result<WsTarget, String>)>,
}
impl WsPolicy {
    pub fn new(
        manifest: &crate::manifest::Manifest,
        settings: &toml::Table,
        loopback: bool,
    ) -> Self {
        let grants = manifest
            .permissions
            .websocket
            .iter()
            .map(|g| {
                let target = (|| {
                    let (host, entered) = match g.host.strip_prefix("setting:") {
                        Some(key) => {
                            let value = match manifest
                                .setting(key)
                                .and_then(|setting| setting.value(settings))
                            {
                                Some(crate::manifest::SettingValue::Text(value)) => value,
                                _ => {
                                    return Err(format!(
                                        "the address in setting {key} is empty or invalid"
                                    ));
                                }
                            };
                            (
                                crate::policy::setting_host(&value)
                                    .trim_end_matches('.')
                                    .to_ascii_lowercase(),
                                true,
                            )
                        }
                        None => (
                            g.host.trim().trim_end_matches('.').to_ascii_lowercase(),
                            false,
                        ),
                    };
                    let host = host.trim_matches(['[', ']']).to_owned();
                    let literal = host.parse::<IpAddr>().ok();
                    if literal.is_none() {
                        crate::policy::check_mqtt_host(
                            &host,
                            if g.tls {
                                crate::manifest::TlsMode::Verify
                            } else {
                                crate::manifest::TlsMode::Plain
                            },
                        )?;
                    }
                    let reach = if !g.tls {
                        Reach::Home
                    } else if entered || literal.is_some() {
                        Reach::Either
                    } else {
                        Reach::Internet
                    };

                    let target = WsTarget {
                        host,
                        port: g.port.unwrap_or(if g.tls { 443 } else { 80 }),
                        tls: g.tls,
                        reach,
                        loopback,
                    };
                    if let Some(ip) = literal {
                        target.allows(ip)?;
                    }
                    Ok(target)
                })();
                (g.name.clone(), target)
            })
            .collect();
        Self { grants }
    }
    pub fn target(&self, name: &str) -> Option<Result<WsTarget, String>> {
        self.grants
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
    }
}

pub(crate) fn open(
    plugin: Arc<Plugin>,
    house: Arc<House>,
    id: u32,
    target: WsTarget,
    url: Url,
    headers: Vec<(String, String)>,
) -> Connection {
    let (tx, rx) = mpsc::channel(OUTBOX_COUNT);
    plugin.queue.register_websocket(id);
    let owner = Some((plugin.clone(), id));
    let shared = Arc::new(Shared {
        connected: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        generation: AtomicU64::new(1),
        out_count: AtomicUsize::new(0),
        out_bytes: AtomicUsize::new(0),
    });
    let task = house.handle.spawn(
        Task {
            id,
            plugin,

            target,
            url,
            headers,
            shared: shared.clone(),
            rx,
            ever_connected: false,
        }
        .run(),
    );
    Connection {
        owner,
        shared,
        tx,
        task: task.abort_handle(),
        handle: house.handle.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Tls,
    Authentication,
    Policy,
    Protocol,
    Transient,
}

fn tls_connector(roots: rustls::RootCertStore) -> tokio_tungstenite::Connector {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("supported TLS versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}

#[cfg(test)]
#[path = "websocket_tls_tests.rs"]
mod tls_tests;

fn classify_handshake_error(error: tokio_tungstenite::tungstenite::Error) -> (FailureKind, String) {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        Error::Http(response) => match response.status().as_u16() {
            401 | 403 => (
                FailureKind::Authentication,
                "WebSocket authentication rejected".into(),
            ),
            408 | 429 | 500..=599 => (
                FailureKind::Transient,
                "WebSocket server temporarily unavailable".into(),
            ),
            _ => (
                FailureKind::Protocol,
                "WebSocket HTTP handshake rejected".into(),
            ),
        },
        Error::Io(ref error)
            if error
                .get_ref()
                .is_some_and(|cause| cause.is::<rustls::Error>()) =>
        {
            (FailureKind::Tls, "TLS handshake rejected".into())
        }
        Error::Io(_) | Error::ConnectionClosed | Error::AlreadyClosed => (
            FailureKind::Transient,
            "WebSocket handshake connection failed".into(),
        ),
        Error::Tls(_) => (FailureKind::Tls, "TLS handshake rejected".into()),
        _ => (
            FailureKind::Protocol,
            "WebSocket protocol handshake rejected".into(),
        ),
    }
}

async fn connect_pinned(
    addresses: &[SocketAddr],
) -> Result<tokio::net::TcpStream, (FailureKind, String)> {
    for address in addresses {
        match tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(address),
        )
        .await
        {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(_)) => {}
            Err(_) => return Err((FailureKind::Transient, "connection timed out".into())),
        }
    }
    Err((FailureKind::Transient, "connection failed".into()))
}

fn checked_addresses(
    ips: impl IntoIterator<Item = IpAddr>,
    port: u16,
    allows: impl Fn(IpAddr) -> Result<(), String>,
) -> Result<Vec<SocketAddr>, (FailureKind, String)> {
    ips.into_iter()
        .map(|ip| {
            allows(ip)
                .map_err(|_| (FailureKind::Policy, "policy denied resolved address".into()))?;
            Ok(SocketAddr::new(ip, port))
        })
        .collect()
}

#[cfg(test)]
mod pinned_address_tests {
    use super::*;

    #[test]
    fn all_resolved_addresses_are_checked_before_any_are_returned() {
        let ips = ["127.0.0.1", "192.0.2.1"].map(|ip| ip.parse().unwrap());
        let checked = checked_addresses(ips, 8123, |ip| {
            if ip.is_loopback() {
                Ok(())
            } else {
                Err("denied".into())
            }
        });
        assert_eq!(
            checked,
            Err((FailureKind::Policy, "policy denied resolved address".into()))
        );
    }

    #[tokio::test]
    async fn connect_falls_back_to_the_next_prechecked_socket_address() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let accepted = listener.local_addr().unwrap();
        let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let refused = unused.local_addr().unwrap();
        drop(unused);
        let server = tokio::spawn(async move { listener.accept().await.unwrap() });
        let stream = connect_pinned(&[refused, accepted]).await.unwrap();
        assert_eq!(stream.peer_addr().unwrap(), accepted);
        let (_, peer) = server.await.unwrap();
        assert_eq!(peer, stream.local_addr().unwrap());
    }

    #[test]
    fn websocket_request_keeps_the_uri_host_and_nondefault_port() {
        let request = "ws://device.example.test:8088/socket?token=x"
            .into_client_request()
            .unwrap();
        assert_eq!(request.uri().host(), Some("device.example.test"));
        assert_eq!(request.uri().port_u16(), Some(8088));
        assert_eq!(request.headers()["host"], "device.example.test:8088");
        assert_eq!(
            request.uri().path_and_query().unwrap().as_str(),
            "/socket?token=x"
        );
    }
}

#[test]
fn handshake_http_classification_does_not_leak_body() {
    use tokio_tungstenite::tungstenite::{Error, http::Response};
    for (status, terminal) in [
        (401, true),
        (403, true),
        (400, true),
        (302, true),
        (408, false),
        (429, false),
        (500, false),
        (503, false),
    ] {
        let response = Response::builder()
            .status(status)
            .body(Some(b"private upstream response".to_vec()))
            .unwrap();
        let (actual, reason) = classify_handshake_error(Error::Http(Box::new(response)));
        assert_eq!(actual == FailureKind::Transient, !terminal);
        assert!(!reason.contains("private upstream"));
        assert_eq!(
            reason.contains("authentication"),
            matches!(status, 401 | 403)
        );
    }
    assert!(
        classify_handshake_error(Error::Io(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset
        )))
        .0 == FailureKind::Transient
    );
}

#[test]
fn wrapped_certificate_errors_are_terminal_tls_failures() {
    use tokio_tungstenite::tungstenite::Error;
    for certificate_error in [
        rustls::CertificateError::UnknownIssuer,
        rustls::CertificateError::NotValidForName,
        rustls::CertificateError::Expired,
    ] {
        let error = Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(certificate_error),
        ));
        let (kind, reason) = classify_handshake_error(error);
        assert_eq!(kind, FailureKind::Tls);
        assert_eq!(reason, "TLS handshake rejected");
    }
}

fn retry_delay(consecutive_failures: usize) -> Duration {
    BACKOFF[consecutive_failures
        .saturating_sub(1)
        .min(BACKOFF.len() - 1)]
}

fn next_retry_delay(consecutive_failures: &mut usize) -> Duration {
    *consecutive_failures = consecutive_failures.saturating_add(1);
    retry_delay(*consecutive_failures)
}

#[cfg(test)]
fn retry_waits(events: impl IntoIterator<Item = bool>) -> Vec<Duration> {
    let mut failures = 0usize;
    events
        .into_iter()
        .filter_map(|connected| {
            if connected {
                failures = 0;
                return None;
            }
            Some(next_retry_delay(&mut failures))
        })
        .collect()
}

async fn wait_for_retry(
    rx: &mut mpsc::Receiver<Command>,
    deadline: tokio::time::Instant,
    shared: &Shared,
) -> bool {
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return false,
            cmd = rx.recv() => match cmd {
                None | Some(Command::Close) => return true,
                Some(Command::Send { message, .. }) => {
                    shared.out_count.fetch_sub(1, Ordering::SeqCst);
                    shared.out_bytes.fetch_sub(message.len(), Ordering::SeqCst);
                }
            }
        }
    }
}

struct Task {
    id: u32,
    plugin: Arc<Plugin>,

    target: WsTarget,
    url: Url,
    headers: Vec<(String, String)>,
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Command>,
    ever_connected: bool,
}
impl Task {
    async fn run(mut self) {
        let mut attempt: usize = 0;
        loop {
            if self.shared.closed.load(Ordering::SeqCst) {
                return;
            }
            let result = self.session().await;
            self.shared.connected.store(false, Ordering::SeqCst);
            let reset_attempt = std::mem::take(&mut self.ever_connected);
            match result {
                Ok(()) => {
                    if self.shared.closed.load(Ordering::SeqCst) {
                        return;
                    }
                    self.status(wit::ConnectionStatus::Disconnected(
                        "connection closed".into(),
                    ));
                    if reset_attempt {
                        attempt = 0;
                    }
                }
                Err((kind, why)) => {
                    self.shared.connected.store(false, Ordering::SeqCst);
                    if kind != FailureKind::Transient {
                        let kind = match kind {
                            FailureKind::Tls => wit::FailureKind::Tls,
                            FailureKind::Authentication => wit::FailureKind::Authentication,
                            FailureKind::Policy => wit::FailureKind::Policy,
                            FailureKind::Protocol => wit::FailureKind::Protocol,
                            FailureKind::Transient => unreachable!(),
                        };
                        self.status(wit::ConnectionStatus::Failed(kind));
                        return;
                    }
                    self.status(wit::ConnectionStatus::Disconnected(why));
                    if reset_attempt {
                        attempt = 0;
                    }
                }
            }
            let wait = next_retry_delay(&mut attempt);
            let deadline = tokio::time::Instant::now() + wait;
            if wait_for_retry(&mut self.rx, deadline, &self.shared).await {
                return;
            }
        }
    }
    fn status(&self, status: wit::ConnectionStatus) {
        if !self.shared.closed.load(Ordering::SeqCst) {
            self.plugin.queue.push_websocket(Delivery::Status {
                conn: self.id,
                generation: self.shared.generation.load(Ordering::SeqCst),
                status,
            });
        }
    }
    fn target_url(&self) -> String {
        self.url.as_str().to_string()
    }
    async fn session(&mut self) -> Result<(), (FailureKind, String)> {
        let host = self.target.host.clone();
        let port = self.target.port;
        let ips = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![ip]
        } else {
            tokio::time::timeout(
                Duration::from_secs(10),
                tokio::net::lookup_host((host.as_str(), port)),
            )
            .await
            .map_err(|_| (FailureKind::Transient, "DNS lookup timed out".into()))?
            .map_err(|_| (FailureKind::Transient, "DNS lookup failed".into()))?
            .map(|a| a.ip())
            .collect()
        };
        let addresses = checked_addresses(ips, port, |ip| {
            self.target
                .allows(ip)
                .map_err(|_| "policy denied resolved address".to_string())
        })?;
        if addresses.is_empty() {
            return Err((
                FailureKind::Transient,
                "DNS lookup returned no addresses".into(),
            ));
        }
        let tcp = connect_pinned(&addresses).await?;
        let mut request = self
            .target_url()
            .into_client_request()
            .map_err(|_| (FailureKind::Protocol, "invalid WebSocket request".into()))?;

        for (name, value) in &self.headers {
            let n = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| (FailureKind::Protocol, "invalid handshake header".into()))?;
            let v = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| (FailureKind::Protocol, "invalid handshake header".into()))?;
            request.headers_mut().insert(n, v);
        }
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let connector = tls_connector(roots);
        let connect = tokio_tungstenite::client_async_tls_with_config(
            request,
            tcp,
            Some(config),
            Some(connector),
        );
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), connect)
            .await
            .map_err(|_| {
                (
                    FailureKind::Transient,
                    "WebSocket handshake timed out".into(),
                )
            })?
            .map_err(classify_handshake_error)?;
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.connected.store(true, Ordering::SeqCst);
        self.ever_connected = true;
        self.plugin.queue.push_websocket(Delivery::Status {
            conn: self.id,
            generation,
            status: wit::ConnectionStatus::Connected,
        });
        loop {
            tokio::select! {
                cmd = self.rx.recv() => match cmd {
                    Some(Command::Close) | None => { let _ = socket.close(None).await; self.shared.connected.store(false, Ordering::SeqCst); return Ok(()); }
                    Some(Command::Send { message, generation: queued_generation }) => { self.shared.out_count.fetch_sub(1, Ordering::SeqCst); self.shared.out_bytes.fetch_sub(message.len(), Ordering::SeqCst); if queued_generation == generation && socket.send(message).await.is_err() { self.shared.connected.store(false, Ordering::SeqCst); return Err((FailureKind::Transient, "send failed".into())); } }
                },
                item = socket.next() => match item {
                    Some(Ok(Message::Text(text))) => { if text.len() > MAX_MESSAGE { return Err((FailureKind::Transient, "message exceeded limit".into())); } self.plugin.queue.push_websocket(Delivery::Message { conn: self.id, generation, message: wit::Message::Text(text.to_string()) }); }
                    Some(Ok(Message::Binary(bytes))) => { if bytes.len() > MAX_MESSAGE { return Err((FailureKind::Transient, "message exceeded limit".into())); } self.plugin.queue.push_websocket(Delivery::Message { conn: self.id, generation, message: wit::Message::Binary(bytes.to_vec()) }); }
                    Some(Ok(Message::Close(_))) | None => { self.shared.connected.store(false, Ordering::SeqCst); return Err((FailureKind::Transient, "peer closed connection".into())); }
                    Some(Err(_)) => { self.shared.connected.store(false, Ordering::SeqCst); return Err((FailureKind::Transient, "receive failed".into())); }
                    _ => {}
                }
            }
        }
    }
}

fn check_url(raw: &str, t: &WsTarget) -> Result<(), HostError> {
    if raw.len() > 4096 || raw.bytes().any(|b| b == b'\r' || b == b'\n') {
        return Err(HostError::Invalid("invalid WebSocket URL".into()));
    }
    let u = Url::parse(raw).map_err(|_| HostError::Invalid("invalid WebSocket URL".into()))?;
    if !u.username().is_empty() || u.password().is_some() || u.fragment().is_some() {
        return Err(HostError::Invalid(
            "WebSocket URLs cannot contain userinfo or fragments".into(),
        ));
    }
    if (t.tls && u.scheme() != "wss") || (!t.tls && u.scheme() != "ws") {
        return Err(HostError::NotPermitted(
            "WebSocket scheme does not match the declared TLS mode".into(),
        ));
    }
    let port = u
        .port_or_known_default()
        .unwrap_or(if t.tls { 443 } else { 80 });
    let url_host = match u.host() {
        Some(url::Host::Domain(host)) => host.trim_end_matches('.').to_ascii_lowercase(),
        Some(url::Host::Ipv4(ip)) => ip.to_string(),
        Some(url::Host::Ipv6(ip)) => ip.to_string(),
        None => String::new(),
    };
    let grant_host = t
        .host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if url_host != grant_host || port != t.port {
        return Err(HostError::NotPermitted(
            "URL host or port differs from the named grant".into(),
        ));
    }
    Ok(())
}
fn check_headers(headers: &[(String, String)]) -> Result<(), HostError> {
    if headers.len() > 32
        || headers
            .iter()
            .map(|(n, v)| n.len() + v.len())
            .sum::<usize>()
            > 8192
    {
        return Err(HostError::Limit(
            "at most 32 headers and 8 KiB total".into(),
        ));
    }
    for (n, v) in headers {
        let lower = n.to_ascii_lowercase();
        if n.is_empty()
            || !n
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || v.bytes().any(|b| b == b'\r' || b == b'\n')
        {
            return Err(HostError::Invalid(
                "invalid WebSocket handshake header".into(),
            ));
        }
        if ["host", "connection", "upgrade", "origin"].contains(&lower.as_str())
            || lower.starts_with("sec-websocket-")
        {
            return Err(HostError::Invalid(
                "header overrides the WebSocket handshake".into(),
            ));
        }
    }
    Ok(())
}
impl crate::bindings::websocket::Host for crate::host::HostState {
    fn connect(
        &mut self,
        grant: String,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<u32, HostError> {
        let target = self
            .ws_policy
            .target(&grant)
            .ok_or_else(|| {
                HostError::NotPermitted("the manifest names no such websocket grant".into())
            })?
            .map_err(HostError::NotPermitted)?;
        if self.websocket_connections.len() >= MAX_CONNECTIONS {
            return Err(HostError::Limit(format!(
                "at most {} WebSocket connections",
                MAX_CONNECTIONS
            )));
        }
        check_url(&url, &target)?;
        check_headers(&headers)?;
        let id = self.plugin.next_websocket_connection();
        let endpoint =
            Url::parse(&url).map_err(|_| HostError::Invalid("invalid WebSocket URL".into()))?;
        let connection = open(
            self.plugin.clone(),
            self.house.clone(),
            id,
            target,
            endpoint,
            headers,
        );
        tracing::debug!(plugin = %self.plugin.id, connection = id, "WebSocket connection requested");
        self.websocket_connections.insert(id, connection);
        Ok(id)
    }
    fn send(&mut self, conn: u32, message: wit::Message) -> Result<(), HostError> {
        self.websocket_connections
            .get(&conn)
            .ok_or_else(|| HostError::Invalid("there is no WebSocket connection".into()))?
            .send(message)
    }
    fn close(&mut self, conn: u32) {
        if let Some(c) = self.websocket_connections.remove(&conn) {
            c.close();
        }
    }
}

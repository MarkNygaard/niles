//! Niles's proxy, the only way a browsing session reaches the internet.
//!
//! It speaks CONNECT and only CONNECT, to port 443 only, and applies
//! `niles-websearch`'s address rules to every connection: the name must
//! be a public one, and every address it resolves to must be public,
//! before any of those addresses is connected to. A redirect is a new
//! connection, checked the same way from scratch. The proxy logs its
//! refusals and nothing it lets through.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use url::Host;

use crate::error::{Error, Result};
use niles_websearch::{public_ip, public_name};

/// How many tunnels the proxy runs at once.
const ROOM: usize = 256;

/// How much of a request head is read, and how long that may take.
const HEAD_MAX: usize = 8 * 1024;
const HEAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long one connect attempt to a checked address may take.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What looks up and dials addresses: the system, or a test fake.
#[async_trait]
pub(crate) trait Network: Send + Sync {
    async fn lookup(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
    async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream>;
}

/// The real network: absolute-name lookup, so the pod's search domains
/// can't turn `niles.default` into a cluster Service, and a plain dial.
pub(crate) struct System;

#[async_trait]
impl Network for System {
    async fn lookup(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        tokio::net::lookup_host((format!("{host}."), port))
            .await
            .map(Vec::from_iter)
    }

    async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream> {
        TcpStream::connect(to).await
    }
}

/// The CONNECT-only proxy the browser sessions reach the internet
/// through.
pub struct Proxy {
    listener: TcpListener,
    network: Arc<dyn Network>,
    room: Arc<Semaphore>,
}

impl Proxy {
    /// Binds the proxy at `at`.
    pub async fn bind(at: SocketAddr) -> Result<Proxy> {
        let listener = TcpListener::bind(at).await.map_err(|e| Error::Listen {
            address: at.to_string(),
            reason: e.to_string(),
        })?;
        Ok(Proxy {
            listener,
            network: Arc::new(System),
            room: Arc::new(Semaphore::new(ROOM)),
        })
    }

    /// The address the proxy listens on.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(|e| Error::Listen {
            address: "?".into(),
            reason: e.to_string(),
        })
    }

    /// Accepts connections forever, one task per connection.
    pub async fn serve(self) {
        loop {
            let Ok((stream, _)) = self.listener.accept().await else {
                continue;
            };
            let network = Arc::clone(&self.network);
            let room = Arc::clone(&self.room);
            tokio::spawn(async move {
                // Full is answered at once: a connection left waiting
                // would look, to the browser, like a proxy gone silent.
                let Ok(_permit) = room.try_acquire() else {
                    tracing::warn!(
                        "[browser] proxy refused a connection: all {ROOM} tunnels are in use"
                    );
                    let mut stream = stream;
                    refuse(
                        &mut stream,
                        "503 Service Unavailable",
                        None,
                        "all tunnels are in use",
                    )
                    .await;
                    return;
                };
                serve_conn(stream, network).await;
            });
        }
    }

    #[cfg(test)]
    fn with_network(listener: TcpListener, network: Arc<dyn Network>) -> Proxy {
        Proxy {
            listener,
            network,
            room: Arc::new(Semaphore::new(ROOM)),
        }
    }
}

/// The per-connection protocol: read the head, check the target, tunnel.
async fn serve_conn(mut stream: TcpStream, network: Arc<dyn Network>) {
    let Ok((head, past)) = read_head(&mut stream).await else {
        refuse(&mut stream, "400 Bad Request", None, "unreadable request").await;
        return;
    };
    let Some(line) = head.lines().next() else {
        refuse(&mut stream, "400 Bad Request", None, "empty request").await;
        return;
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        refuse(
            &mut stream,
            "400 Bad Request",
            None,
            "malformed request line",
        )
        .await;
        return;
    };
    if method != "CONNECT" {
        stream
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nAllow: CONNECT\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .await
            .ok();
        // The refusal carries the target's host when one parses.
        if let Ok(url) = url::Url::parse(target)
            && let Some(host) = url.host_str()
        {
            tracing::warn!("[browser] proxy refused {host}: only https");
        }
        return;
    }
    let Some(target) = parse_target(target) else {
        refuse(&mut stream, "400 Bad Request", None, "malformed target").await;
        return;
    };
    let checked = match check(&network, &target).await {
        Ok(checked) => checked,
        Err(answer) => {
            refuse(
                &mut stream,
                answer.status,
                Some((target.host.as_str(), target.port)),
                answer.reason,
            )
            .await;
            return;
        }
    };
    let mut upstream = match dial(&network, checked).await {
        Some(upstream) => upstream,
        None => {
            refuse(
                &mut stream,
                "502 Bad Gateway",
                Some((target.host.as_str(), target.port)),
                "no checked address answered",
            )
            .await;
            return;
        }
    };
    if stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    // Bytes read past the head belong to the tunneled protocol: they
    // came with the CONNECT and must reach the far end first.
    if !past.is_empty() && stream.write_all(&past).await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
}

/// Reads until the end of the request head, at most `HEAD_MAX` bytes
/// within `HEAD_TIMEOUT`, keeping any bytes read past the head: they
/// belong to the tunneled protocol.
async fn read_head(stream: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    tokio::time::timeout(HEAD_TIMEOUT, async {
        loop {
            if buf.len() >= HEAD_MAX {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "head too big"));
            }
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "head cut"));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = find(&buf, b"\r\n\r\n") {
                let head = String::from_utf8(buf[..end + 4].to_vec())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not utf8"))?;
                return Ok((head, buf[end + 4..].to_vec()));
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "head slow"))?
}

/// The first index of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The target of a CONNECT, split on its last colon, IPv6 in brackets,
/// `@` and `/` refused.
fn parse_target(target: &str) -> Option<Target> {
    if target.contains('@') || target.contains('/') {
        return None;
    }
    let (host, port) = if let Some(inner) = target.strip_prefix('[') {
        let (host, port) = inner.split_once("]:")?;
        (host.to_string(), port)
    } else {
        let (host, port) = target.rsplit_once(':')?;
        (host.to_string(), port)
    };
    let port = port.parse::<u16>().ok()?;
    Some(Target { host, port })
}

/// A parsed CONNECT target.
struct Target {
    host: String,
    port: u16,
}

/// Why a target was refused: the status line and the words for the log.
struct Refused {
    status: &'static str,
    reason: &'static str,
}

/// Applies the address rules. On success, the checked addresses to
/// dial, in the order they resolved.
async fn check(
    network: &Arc<dyn Network>,
    target: &Target,
) -> std::result::Result<Vec<SocketAddr>, Refused> {
    if target.port != 443 {
        return Err(Refused {
            status: "403 Forbidden",
            reason: "only port 443",
        });
    }
    // A bracketed CONNECT target leaves its brackets in parse_target's
    // hands; `Host::parse` wants them back for an IPv6 literal.
    let host = if target.host.contains(':') && !target.host.starts_with('[') {
        format!("[{}]", target.host)
    } else {
        target.host.clone()
    };
    match Host::parse(&host) {
        Ok(Host::Domain(name)) => check_name(network, &name, target.port).await,
        Ok(Host::Ipv4(ip)) => {
            check_ip(ip.into()).map(|_| vec![SocketAddr::new(ip.into(), target.port)])
        }
        Ok(Host::Ipv6(ip)) => {
            check_ip(ip.into()).map(|_| vec![SocketAddr::new(ip.into(), target.port)])
        }
        Err(_) => Err(Refused {
            status: "400 Bad Request",
            reason: "unparseable host",
        }),
    }
}

/// A name's rules: public, and every resolved address public too.
async fn check_name(
    network: &Arc<dyn Network>,
    name: &str,
    port: u16,
) -> std::result::Result<Vec<SocketAddr>, Refused> {
    if !public_name(name) {
        return Err(Refused {
            status: "403 Forbidden",
            reason: "never on the public internet",
        });
    }
    let addresses = network.lookup(name, port).await.map_err(|e| {
        tracing::debug!("[browser] proxy lookup of {name} failed: {e}");
        Refused {
            status: "502 Bad Gateway",
            reason: "lookup failed",
        }
    })?;
    if addresses.is_empty() {
        return Err(Refused {
            status: "502 Bad Gateway",
            reason: "no addresses",
        });
    }
    if let Some(ip) = addresses.iter().map(|a| a.ip()).find(|ip| !public_ip(*ip)) {
        tracing::debug!("[browser] proxy refused {name}: {ip} is inside the network");
        return Err(Refused {
            status: "403 Forbidden",
            reason: "resolves inside the network",
        });
    }
    Ok(addresses)
}

/// An address's rule: on the public internet.
fn check_ip(ip: std::net::IpAddr) -> std::result::Result<(), Refused> {
    if public_ip(ip) {
        Ok(())
    } else {
        Err(Refused {
            status: "403 Forbidden",
            reason: "never on the public internet",
        })
    }
}

/// Dials the checked addresses in order. `None` when none answered.
async fn dial(network: &Arc<dyn Network>, checked: Vec<SocketAddr>) -> Option<TcpStream> {
    for address in checked {
        let Ok(stream) = tokio::time::timeout(CONNECT_TIMEOUT, network.connect(address)).await
        else {
            continue;
        };
        if let Ok(stream) = stream {
            return Some(stream);
        }
    }
    None
}

/// Answers a refusal: the short status, `Connection: close`, no body,
/// and the one log line that names the target.
async fn refuse(stream: &mut TcpStream, status: &str, target: Option<(&str, u16)>, reason: &str) {
    if let Some((host, port)) = target {
        tracing::warn!("[browser] proxy refused {host}:{port}: {reason}");
    }
    stream
        .write_all(
            format!("HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
                .as_bytes(),
        )
        .await
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A fake network: names resolve to scripted answers, and every
    /// dial lands on the local echo listener while recording the
    /// address it was asked for.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<HashMap<String, Vec<IpAddr>>>,
        lookups: Mutex<HashMap<String, usize>>,
        dialed: Mutex<Vec<SocketAddr>>,
        echo: Mutex<Option<SocketAddr>>,
    }

    #[async_trait]
    impl Network for Fake {
        async fn lookup(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
            let key = format!("{host}:{port}");
            *self
                .lookups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.clone())
                .or_default() += 1;
            Ok(self
                .answers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect())
        }

        async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream> {
            self.dialed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(to);
            let echo = *self
                .echo
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .expect("echo address set");
            TcpStream::connect(echo).await
        }
    }

    /// Sends back whatever arrives, for the tunnel tests.
    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept");
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if stream.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        addr
    }

    /// A proxy over a fake network, serving.
    async fn proxy(answers: Vec<(&str, Vec<IpAddr>)>) -> (SocketAddr, Arc<Fake>) {
        let echo = echo_server().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let fake = Arc::new(Fake {
            answers: Mutex::new(
                answers
                    .into_iter()
                    .map(|(name, ips)| (name.to_string(), ips))
                    .collect(),
            ),
            echo: Mutex::new(Some(echo)),
            ..Fake::default()
        });
        tokio::spawn(Proxy::with_network(listener, fake.clone()).serve());
        (addr, fake)
    }

    /// One CONNECT `target`, `extra` written with the head, answered
    /// with the response head, the bytes read past it, and the stream.
    async fn ask(proxy: SocketAddr, target: &str, extra: &[u8]) -> (String, Vec<u8>, TcpStream) {
        let mut stream = TcpStream::connect(proxy).await.expect("connect");
        let mut request =
            format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").into_bytes();
        request.extend_from_slice(extra);
        stream.write_all(&request).await.expect("write");
        let mut got = Vec::new();
        let mut buf = [0u8; 1024];
        let end = loop {
            let n = stream.read(&mut buf).await.expect("read");
            assert!(n > 0, "the proxy answered");
            got.extend_from_slice(&buf[..n]);
            if let Some(end) = find(&got, b"\r\n\r\n") {
                break end + 4;
            }
        };
        (
            String::from_utf8(got[..end].to_vec()).expect("utf8"),
            got[end..].to_vec(),
            stream,
        )
    }

    /// Reads `n` bytes, starting with any the head read already carried.
    async fn read_n(past: Vec<u8>, stream: &mut TcpStream, n: usize) -> Vec<u8> {
        let mut out = past;
        let mut buf = [0u8; 1024];
        while out.len() < n {
            let got = stream.read(&mut buf).await.expect("read");
            assert!(got > 0, "the stream ended early");
            out.extend_from_slice(&buf[..got]);
        }
        out.truncate(n);
        out
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("ip")
    }

    #[tokio::test]
    async fn refuses_private_loopback_link_local_and_cluster_addresses() {
        let (addr, fake) = proxy(vec![]).await;
        let literals = [
            "127.0.0.1:443",
            "10.0.0.5:443",
            "192.168.1.1:443",
            "169.254.169.254:443",
            "100.64.0.1:443",
            "10.96.0.1:443",
            "[::1]:443",
            "[fe80::1]:443",
            "[fd00::1]:443",
            "[::ffff:10.0.0.1]:443",
        ];
        for target in literals {
            let (head, _, _) = ask(addr, target, &[]).await;
            assert!(head.starts_with("HTTP/1.1 403"), "{target}: {head}");
        }
        assert!(
            fake.dialed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "a refused literal is never dialed"
        );
    }

    #[tokio::test]
    async fn refuses_names_never_on_the_public_internet_before_any_lookup() {
        let (addr, fake) = proxy(vec![]).await;
        let names = [
            "localhost:443",
            "niles:443",
            "printer.local:443",
            "kubernetes.default.svc.cluster.local:443",
            "kubernetes.default.svc:443",
            "metadata.google.internal:443",
        ];
        for target in names {
            let (head, _, _) = ask(addr, target, &[]).await;
            assert!(head.starts_with("HTTP/1.1 403"), "{target}: {head}");
        }
        assert!(
            fake.lookups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .all(|count| *count == 0),
            "a refused name is never looked up"
        );
    }

    #[tokio::test]
    async fn refuses_a_name_that_resolves_inside() {
        let (addr, _) = proxy(vec![
            ("inside.example:443", vec![ip("10.0.0.5")]),
            (
                "mixed.example:443",
                vec![ip("93.184.216.34"), ip("10.0.0.5")],
            ),
            ("ula.example:443", vec![ip("fd00::1")]),
        ])
        .await;
        for target in ["inside.example:443", "mixed.example:443", "ula.example:443"] {
            let (head, _, _) = ask(addr, target, &[]).await;
            assert!(head.starts_with("HTTP/1.1 403"), "{target}: {head}");
        }
    }

    #[tokio::test]
    async fn checks_every_connection_again_as_a_redirect_makes() {
        let (addr, fake) = proxy(vec![("shop.example:443", vec![ip("93.184.216.34")])]).await;
        // A first, allowed connection tunnels both ways.
        let (head, past, mut stream) = ask(addr, "shop.example:443", b"ping").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let echoed = tokio::time::timeout(Duration::from_secs(5), read_n(past, &mut stream, 4))
            .await
            .expect("echo in time");
        assert_eq!(echoed, b"ping");
        drop(stream);

        // A new connection to a private name is refused, even though a
        // public connection just went through.
        fake.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert("inside.example:443".into(), vec![ip("10.0.0.5")]);
        let (head, _, _) = ask(addr, "inside.example:443", &[]).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        // The same name rebinding to a private address is refused too:
        // no allowance is kept between connections.
        fake.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert("shop.example:443".into(), vec![ip("10.0.0.5")]);
        let (head, _, _) = ask(addr, "shop.example:443", &[]).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");
    }

    #[tokio::test]
    async fn connects_to_the_address_it_checked() {
        let (addr, fake) = proxy(vec![(
            "one.example:443",
            vec![ip("93.184.216.34"), ip("93.184.216.35")],
        )])
        .await;
        let (head, _, _) = ask(addr, "one.example:443", &[]).await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(
            *fake
                .lookups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get("one.example:443")
                .expect("one lookup"),
            1
        );
        assert_eq!(
            *fake
                .dialed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .first()
                .expect("one dial"),
            SocketAddr::new(ip("93.184.216.34"), 443)
        );
    }

    #[tokio::test]
    async fn takes_connect_to_443_only() {
        let (addr, fake) = proxy(vec![("shop.example:443", vec![ip("93.184.216.34")])]).await;
        let (head, _, _) = ask(addr, "shop.example:80", &[]).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET http://shop.example/ HTTP/1.1\r\n\r\n")
            .await
            .expect("write");
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await.expect("read");
        let head = String::from_utf8(buf[..n].to_vec()).expect("utf8");
        assert!(head.starts_with("HTTP/1.1 405"), "{head}");
        assert!(head.contains("Allow: CONNECT"), "{head}");

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.write_all(b"BLAH\r\n\r\n").await.expect("write");
        let n = stream.read(&mut buf).await.expect("read");
        let head = String::from_utf8(buf[..n].to_vec()).expect("utf8");
        assert!(head.starts_with("HTTP/1.1 400"), "{head}");
        assert!(
            fake.dialed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "nothing was dialed"
        );
    }

    #[tokio::test]
    async fn a_full_proxy_answers_503_and_tunnels_again_once_free() {
        use tokio::io::AsyncWriteExt;
        let echo = echo_server().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let fake = Arc::new(Fake {
            answers: Mutex::new(
                [("shop.example:443", vec![ip("93.184.216.34")])]
                    .into_iter()
                    .map(|(name, ips)| (name.to_string(), ips))
                    .collect(),
            ),
            echo: Mutex::new(Some(echo)),
            ..Fake::default()
        });
        // One tunnel only: the second connection must be refused at once.
        let proxy = Proxy {
            listener,
            network: fake,
            room: Arc::new(Semaphore::new(1)),
        };
        tokio::spawn(proxy.serve());

        let (head, _, mut held) = ask(addr, "shop.example:443", &[]).await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");

        let (head, _, _) = ask(addr, "shop.example:443", &[]).await;
        assert!(head.starts_with("HTTP/1.1 503"), "{head}");

        // Closing the tunnel end to end frees its permit: the FIN makes
        // the proxy shut the far side, and the echo's close ends the
        // task that held the permit.
        held.shutdown().await.expect("shutdown");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let (head, _, _) = ask(addr, "shop.example:443", &[]).await;
            if head.starts_with("HTTP/1.1 200") {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the freed tunnel was not used again: {head}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn forwards_what_came_after_the_head() {
        let (addr, _) = proxy(vec![("shop.example:443", vec![ip("93.184.216.34")])]).await;
        // "ping" rides in the same write as the head: it must reach the
        // far end even though it arrived with the CONNECT.
        let (head, past, mut stream) = ask(addr, "shop.example:443", b"ping").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let echoed = tokio::time::timeout(Duration::from_secs(5), read_n(past, &mut stream, 4))
            .await
            .expect("echo in time");
        assert_eq!(echoed, b"ping");
    }
}

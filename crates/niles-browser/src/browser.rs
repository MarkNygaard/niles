//! The browser itself: discovery of its DevTools websocket, and opening
//! sessions on it.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpStream;
use url::Url;

use crate::cdp::Connection;
use crate::error::{Error, Result};
use crate::session::Session;
use crate::tab::{Driver, Tab};

/// How long discovery and the websocket connect may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A connection to one headless Chromium. Every session it opens is a
/// browser context of its own, reaching the internet only through
/// Niles's proxy.
pub struct Browser {
    conn: Arc<Connection>,
    proxy: String,
    local: IpAddr,
    product: String,
}

impl Browser {
    /// Finds the browser at `devtools` and connects to its DevTools
    /// websocket. `proxy_port` is where Niles's proxy will listen on
    /// the address the browser reaches Niles at.
    pub async fn connect(devtools: SocketAddr, proxy_port: u16) -> Result<Browser> {
        let version = discovery(devtools).await?;
        let path = socket_path(&version)?;
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(devtools))
            .await
            .map_err(|_| Error::Unreachable {
                address: devtools.to_string(),
                reason: "timed out connecting to DevTools".into(),
            })?
            .map_err(|e| Error::Unreachable {
                address: devtools.to_string(),
                reason: e.to_string(),
            })?;
        let local = stream.local_addr().map_err(|e| Error::Unreachable {
            address: devtools.to_string(),
            reason: e.to_string(),
        })?;
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(64 << 20))
            .max_frame_size(Some(16 << 20));
        let (ws, _) = tokio_tungstenite::client_async_with_config(
            format!("ws://{devtools}{path}"),
            stream,
            Some(config),
        )
        .await
        .map_err(|e| Error::Unreachable {
            address: devtools.to_string(),
            reason: e.to_string(),
        })?;
        let conn = Connection::open(ws);
        let product = version_product(&conn)
            .await
            .map_err(|e| Error::Unreachable {
                address: devtools.to_string(),
                reason: e.to_string(),
            })?;
        Ok(Browser {
            conn,
            proxy: format!("http://{}", SocketAddr::new(local.ip(), proxy_port)),
            local: local.ip(),
            product,
        })
    }

    /// The address the browser reaches Niles at: the local address of
    /// this connection. Niles's proxy binds it, and nothing else.
    pub fn local_ip(&self) -> IpAddr {
        self.local
    }

    /// What Chromium says it is, for the log line when Niles connects.
    pub fn product(&self) -> &str {
        &self.product
    }

    /// One session: a fresh browser context with one page, every
    /// connection of it through Niles's proxy.
    async fn new_session(&self) -> Result<Arc<dyn Tab>> {
        let context = self
            .conn
            .call(
                "Target.createBrowserContext",
                json!({
                    "disposeOnDetach": true,
                    "proxyServer": self.proxy,
                    "proxyBypassList": "<-loopback>"
                }),
                None,
            )
            .await?
            .get("browserContextId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Target.createBrowserContext",
                reason: "no browserContextId".into(),
            })?;

        let made = self.session_in(&context).await;
        if made.is_err() {
            // Best effort: no context leaks when a later step failed.
            let _ = self
                .conn
                .call(
                    "Target.disposeBrowserContext",
                    json!({"browserContextId": context}),
                    None,
                )
                .await;
        }
        made
    }

    /// The session's steps after the context exists: downloads denied,
    /// one blank page, attached flat.
    async fn session_in(&self, context: &str) -> Result<Arc<dyn Tab>> {
        self.conn
            .call(
                "Browser.setDownloadBehavior",
                json!({"behavior": "deny", "browserContextId": context}),
                None,
            )
            .await?;
        let target = self
            .conn
            .call(
                "Target.createTarget",
                json!({"url": "about:blank", "browserContextId": context}),
                None,
            )
            .await?
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Target.createTarget",
                reason: "no targetId".into(),
            })?;
        let session = self
            .conn
            .call(
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
                None,
            )
            .await?
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Answer {
                method: "Target.attachToTarget",
                reason: "no sessionId".into(),
            })?;
        let started: Arc<dyn Tab> =
            Session::start(Arc::clone(&self.conn), context.to_string(), target, session).await?;
        Ok(started)
    }
}

/// `Browser.getVersion`'s product, or a shape complaint.
async fn version_product(conn: &Connection) -> Result<String> {
    conn.call("Browser.getVersion", Value::Null, None)
        .await?
        .get("product")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Answer {
            method: "Browser.getVersion",
            reason: "no product string".into(),
        })
}

/// `GET /json/version`, without a proxy and with a short timeout. The
/// host is an IP, which Chromium's Host-header check accepts.
async fn discovery(devtools: SocketAddr) -> Result<Value> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| Error::Unreachable {
            address: devtools.to_string(),
            reason: e.to_string(),
        })?;
    let text = client
        .get(format!("http://{devtools}/json/version"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Error::Unreachable {
            address: devtools.to_string(),
            reason: e.to_string(),
        })?
        .text()
        .await
        .map_err(|e| Error::Unreachable {
            address: devtools.to_string(),
            reason: e.to_string(),
        })?;
    serde_json::from_str(&text).map_err(|_| Error::Answer {
        method: "GET /json/version",
        reason: "not a version document".into(),
    })
}

/// The websocket path out of the version document: only
/// `/devtools/browser/…` is a browser-level socket.
fn socket_path(version: &Value) -> Result<String> {
    let raw = version
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Answer {
            method: "GET /json/version",
            reason: "no webSocketDebuggerUrl".into(),
        })?;
    let url = Url::parse(raw).map_err(|_| Error::Unreachable {
        address: raw.to_string(),
        reason: "webSocketDebuggerUrl isn't a URL".into(),
    })?;
    let path = url.path();
    if path.starts_with("/devtools/browser/") {
        Ok(path.to_string())
    } else {
        Err(Error::Unreachable {
            address: raw.to_string(),
            reason: "not a browser-level DevTools socket".into(),
        })
    }
}

#[async_trait::async_trait]
impl Driver for Browser {
    async fn session(&self) -> Result<Arc<dyn Tab>> {
        self.new_session().await
    }

    async fn restart(&self) -> Result<()> {
        match self.conn.call("Browser.close", Value::Null, None).await {
            // The browser exits when it closes: those mean it did.
            Ok(_) | Err(Error::Closed) | Err(Error::Timeout { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn closed(&self) -> bool {
        self.conn.closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first index of `needle` in `hay`.
    fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    /// A test browser on one port: HTTP for `/json/version`, a
    /// websocket otherwise, answering `Browser.getVersion`.
    async fn test_browser(socket_path: &str) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let socket_path = socket_path.to_string();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let socket_path = socket_path.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    // Peek, don't consume: the websocket handshake is
                    // handed to accept_async whole.
                    let mut peeked = vec![0u8; 4096];
                    let n = loop {
                        let n = stream.peek(&mut peeked).await.expect("peek");
                        if n > 0 && peeked[..n].contains(&b'\n') {
                            break n;
                        }
                    };
                    let head = String::from_utf8_lossy(&peeked[..n]).into_owned();
                    if head.starts_with("GET /json/version ") {
                        // The whole head, so it can be consumed exactly.
                        let n = loop {
                            let n = stream.peek(&mut peeked).await.expect("peek");
                            if find_sub(&peeked[..n], b"\r\n\r\n").is_some() {
                                break n;
                            }
                        };
                        let head = String::from_utf8_lossy(&peeked[..n]).into_owned();
                        let body = format!(
                            r#"{{"webSocketDebuggerUrl": "ws://127.0.0.1:9222{socket_path}"}}"#
                        );
                        let end = head.find("\r\n\r\n").expect("head end") + 4;
                        let mut take = vec![0u8; end];
                        stream.read_exact(&mut take).await.expect("consume head");
                        stream
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                    body.len(),
                                    body
                                )
                                .as_bytes(),
                            )
                            .await
                            .expect("write");
                        return;
                    }
                    // Any other GET is the websocket handshake, read by
                    // accept_async itself.
                    let ws = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("accept ws");
                    let (mut sink, mut stream) = ws.split();
                    use futures_util::{SinkExt, StreamExt};
                    while let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                        stream.next().await
                    {
                        let frame: Value = serde_json::from_str(&text).expect("json");
                        let id = frame["id"].as_u64().expect("id");
                        sink.send(tokio_tungstenite::tungstenite::Message::Text(
                            json!({"id": id, "result": {"product": "HeadlessChrome/158.0.0.0"}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .expect("send");
                    }
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn connect_discovers_the_socket_and_reads_the_product() {
        let addr = test_browser("/devtools/browser/abc123").await;
        let browser = Browser::connect(addr, 3128).await.expect("connect");
        assert_eq!(browser.product(), "HeadlessChrome/158.0.0.0");
        assert_eq!(browser.local_ip(), addr.ip());
    }

    #[tokio::test]
    async fn a_socket_that_isnt_the_browsers_is_unreachable() {
        let addr = test_browser("/devtools/page/abc123").await;
        match Browser::connect(addr, 3128).await {
            Err(Error::Unreachable { reason, .. }) => {
                assert!(reason.contains("browser-level"), "{reason}");
            }
            other => panic!(
                "expected Unreachable, got {}",
                other.err().expect("an error")
            ),
        }
    }
}

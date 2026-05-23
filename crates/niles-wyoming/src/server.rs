//! Wyoming TCP server.
//!
//! v0.1: accepts connections, parses incoming events, forwards them
//! onto an mpsc channel for the caller to handle. Writing back to
//! the satellite (TTS audio, intent responses) lands in a later PR.

use crate::codec::WyomingReader;
use crate::error::Result;
use crate::event::Event;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Capacity of the incoming-event channel. A bounded channel
/// applies backpressure to the per-connection reader (and through
/// it, to the TCP socket and ultimately the satellite) if the
/// consumer falls behind, instead of letting memory grow without
/// bound. 1024 events ≈ 20s of audio at typical chunk rates —
/// plenty of headroom for transient consumer slow-downs.
const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Drop a connection that hasn't produced an event in this long.
/// Wyoming has ping/pong for keepalive, so a healthy satellite
/// sends something at least every minute or two. 10 minutes is a
/// generous floor that still reclaims dead sockets.
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// Sleep this long after a transient `accept()` error to avoid
/// spinning a hot loop if the error condition (e.g. FD exhaustion)
/// is persistent.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// One inbound event tagged with which connection it came from.
/// The remote address is convenient for logging while we wait to
/// introduce a real session/satellite identity.
#[derive(Debug)]
pub struct IncomingEvent {
    pub from: SocketAddr,
    pub event: Event,
}

/// Wyoming server. Owns the listener and the sender sides of the
/// inbound-event and disconnect channels; spawns a per-connection
/// task that reads events from the socket and forwards them, plus
/// a single per-connection disconnect notification when the task
/// exits.
pub struct WyomingServer {
    listener: TcpListener,
    events_tx: mpsc::Sender<IncomingEvent>,
    /// Unbounded so that a slow consumer (or one that dropped the
    /// receiver entirely — see `wyoming-tap`) never blocks the
    /// accept loop. Each notification is a single `SocketAddr`,
    /// and connections close one at a time per peer, so the queue
    /// stays tiny in practice.
    disconnects_tx: mpsc::UnboundedSender<SocketAddr>,
}

impl WyomingServer {
    /// Bind to `addr` and return the server plus the receiver ends
    /// of the event and disconnect channels.
    ///
    /// The disconnect channel fires exactly one `SocketAddr` per
    /// closed connection — any exit path of the per-connection
    /// reader: idle timeout, peer EOF, parse error, or the events
    /// receiver being dropped. Callers that don't need disconnect
    /// notifications can drop the returned disconnect receiver; the
    /// server's `send` then silently fails and never blocks.
    pub async fn bind(
        addr: SocketAddr,
    ) -> Result<(
        Self,
        mpsc::Receiver<IncomingEvent>,
        mpsc::UnboundedReceiver<SocketAddr>,
    )> {
        let listener = TcpListener::bind(addr).await?;
        let (events_tx, events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let (disconnects_tx, disconnects_rx) = mpsc::unbounded_channel();
        info!("niles-wyoming listening on tcp://{addr}");
        Ok((
            Self {
                listener,
                events_tx,
                disconnects_tx,
            },
            events_rx,
            disconnects_rx,
        ))
    }

    /// Run the accept loop forever. Each accepted connection gets its
    /// own task that reads events until the peer disconnects, hits an
    /// idle timeout, or sends malformed input. The function only
    /// returns when its task is cancelled (e.g. by `JoinHandle::abort`
    /// or a future shutdown signal).
    pub async fn run(self) {
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    warn!("accept error: {e}");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                    continue;
                }
            };
            debug!("new Wyoming connection from {peer}");
            let events_tx = self.events_tx.clone();
            let disconnects_tx = self.disconnects_tx.clone();
            tokio::spawn(async move {
                handle_connection(stream, peer, events_tx).await;
                // Best-effort disconnect notification. If the
                // consumer dropped its receiver (e.g. wyoming-tap
                // doesn't care), `send` returns `SendError(Closed)`
                // and we move on — the server is not blocked.
                let _ = disconnects_tx.send(peer);
            });
        }
    }
}

async fn handle_connection(stream: TcpStream, peer: SocketAddr, tx: mpsc::Sender<IncomingEvent>) {
    let mut reader = WyomingReader::new(stream);
    loop {
        let event = match tokio::time::timeout(IDLE_TIMEOUT, reader.read_event()).await {
            Err(_) => {
                warn!(
                    "Wyoming idle timeout for {peer} ({}s with no events)",
                    IDLE_TIMEOUT.as_secs()
                );
                return;
            }
            Ok(Ok(Some(event))) => event,
            Ok(Ok(None)) => {
                debug!("{peer} closed connection");
                return;
            }
            Ok(Err(e)) => {
                warn!("Wyoming parse error from {peer}: {e}");
                return;
            }
        };
        if tx.send(IncomingEvent { from: peer, event }).await.is_err() {
            debug!("event consumer dropped — closing {peer}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::WyomingWriter;
    use crate::event::EventKind;
    use serde_json::Value;
    use std::time::Duration;
    use tokio::net::TcpStream;

    /// End-to-end: bind on an OS-assigned port, connect a client,
    /// send three events, confirm the server task forwards them.
    #[tokio::test]
    async fn server_forwards_events_to_channel() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut rx, _disconnects) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();

        // Run the accept loop in the background.
        let _server_task = tokio::spawn(async move { server.run().await });

        // Client: send describe, audio-chunk (with payload), audio-stop.
        let stream = TcpStream::connect(bound).await.unwrap();
        let mut writer = WyomingWriter::new(stream);
        writer
            .write_event(&Event {
                kind: EventKind::Describe,
                data: Value::Null,
                payload: Vec::new(),
                version: None,
            })
            .await
            .unwrap();
        writer
            .write_event(&Event {
                kind: EventKind::AudioChunk,
                data: serde_json::json!({"rate": 16000}),
                payload: vec![0xaa, 0xbb, 0xcc],
                version: None,
            })
            .await
            .unwrap();
        writer
            .write_event(&Event {
                kind: EventKind::AudioStop,
                data: Value::Null,
                payload: Vec::new(),
                version: None,
            })
            .await
            .unwrap();

        // Read the three events from the server side, with a short
        // overall timeout so a hung test fails loudly.
        let collect = async {
            let mut events = Vec::new();
            for _ in 0..3 {
                events.push(rx.recv().await.expect("event"));
            }
            events
        };
        let events = tokio::time::timeout(Duration::from_secs(2), collect)
            .await
            .expect("timed out waiting for events");

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event.kind, EventKind::Describe);
        assert_eq!(events[1].event.kind, EventKind::AudioChunk);
        assert_eq!(events[1].event.payload, vec![0xaa, 0xbb, 0xcc]);
        assert_eq!(events[2].event.kind, EventKind::AudioStop);
        // All from the same client connection.
        assert_eq!(events[0].from, events[1].from);
        assert_eq!(events[1].from, events[2].from);
    }

    /// End-to-end: client closes the TCP socket → server emits a
    /// disconnect notification carrying that peer's address.
    #[tokio::test]
    async fn disconnect_notification_fires_on_peer_close() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, mut disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let _server_task = tokio::spawn(async move { server.run().await });

        // Open a connection, send one event so the server learns the
        // peer address from the wire (and we can confirm it matches),
        // then drop the stream to trigger EOF on the server.
        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let mut writer = WyomingWriter::new(stream);
        writer
            .write_event(&Event {
                kind: EventKind::Ping,
                data: Value::Null,
                payload: Vec::new(),
                version: None,
            })
            .await
            .unwrap();
        let ev = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("event recv timed out")
            .expect("event");
        assert_eq!(ev.from, client_peer);

        // Closing the writer drops the underlying TcpStream → server
        // sees clean EOF → per-connection task exits → disconnect fires.
        drop(writer);

        let dropped = tokio::time::timeout(Duration::from_secs(2), disconnects_rx.recv())
            .await
            .expect("disconnect recv timed out")
            .expect("disconnect");
        assert_eq!(
            dropped, client_peer,
            "disconnect should carry the same peer address as the events"
        );
    }
}

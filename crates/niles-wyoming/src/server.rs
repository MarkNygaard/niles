//! Wyoming TCP server.
//!
//! v0.1: accepts connections, parses incoming events, forwards them
//! onto an mpsc channel for the caller to handle. Writing back to
//! the satellite (TTS audio, intent responses) lands in a later PR.

use crate::codec::WyomingReader;
use crate::error::Result;
use crate::event::Event;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// One inbound event tagged with which connection it came from.
/// The remote address is convenient for logging while we wait to
/// introduce a real session/satellite identity.
#[derive(Debug)]
pub struct IncomingEvent {
    pub from: SocketAddr,
    pub event: Event,
}

/// Wyoming server. Owns the listener and a sender side of the
/// incoming-event channel; spawns a per-connection task that reads
/// events from the socket and forwards them.
pub struct WyomingServer {
    listener: TcpListener,
    tx: mpsc::UnboundedSender<IncomingEvent>,
}

impl WyomingServer {
    /// Bind to `addr` and return the server plus the receiver end of
    /// the event channel.
    pub async fn bind(addr: SocketAddr) -> Result<(Self, mpsc::UnboundedReceiver<IncomingEvent>)> {
        let listener = TcpListener::bind(addr).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        info!("niles-wyoming listening on tcp://{addr}");
        Ok((Self { listener, tx }, rx))
    }

    /// Run the accept loop until the listener errors. Each accepted
    /// connection gets its own task that reads events until the
    /// peer disconnects.
    pub async fn run(self) -> Result<()> {
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    warn!("accept error: {e}");
                    continue;
                }
            };
            debug!("new Wyoming connection from {peer}");
            let tx = self.tx.clone();
            tokio::spawn(handle_connection(stream, peer, tx));
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    tx: mpsc::UnboundedSender<IncomingEvent>,
) {
    let mut reader = WyomingReader::new(stream);
    loop {
        match reader.read_event().await {
            Ok(Some(event)) => {
                if tx.send(IncomingEvent { from: peer, event }).is_err() {
                    debug!("event consumer dropped — closing {peer}");
                    return;
                }
            }
            Ok(None) => {
                debug!("{peer} closed connection");
                return;
            }
            Err(e) => {
                warn!("Wyoming parse error from {peer}: {e}");
                return;
            }
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
        let (server, mut rx) = WyomingServer::bind(addr).await.unwrap();
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
}

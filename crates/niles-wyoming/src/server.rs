//! Wyoming TCP server.
//!
//! v0.1: accepts connections, parses incoming events, forwards them
//! onto an mpsc channel for the caller to handle, and supports
//! sending events (including framed PCM audio) back to any connected
//! peer via [`WyomingSender`].

use crate::codec::{WyomingReader, WyomingWriter};
use crate::error::{Result, SendError};
use crate::event::{Event, EventKind};
use crate::session::AudioFormat;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
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

/// Bounded so a stalled satellite socket applies backpressure
/// instead of growing memory.
const OUTBOUND_CHANNEL_CAPACITY: usize = 256;

/// 2048 PCM bytes per chunk — ~64ms at 16 kHz mono 16-bit, small
/// enough for responsive streaming.
const AUDIO_CHUNK_BYTES: usize = 2048;

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
    /// Maps connected peer addresses to their outbound event
    /// sender. The corresponding receiver drives a per-connection
    /// writer task. Removed on disconnect so subsequent sends
    /// return `SendError::NotConnected`.
    outbound: Arc<Mutex<HashMap<SocketAddr, mpsc::Sender<Event>>>>,
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
        let outbound = Arc::new(Mutex::new(HashMap::new()));
        info!("niles-wyoming listening on tcp://{addr}");
        Ok((
            Self {
                listener,
                events_tx,
                disconnects_tx,
                outbound,
            },
            events_rx,
            disconnects_rx,
        ))
    }

    /// Returns a clonable handle that can send events (including
    /// framed audio sequences) to any currently connected peer.
    pub fn sender(&self) -> WyomingSender {
        WyomingSender {
            outbound: Arc::clone(&self.outbound),
        }
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
            let outbound = Arc::clone(&self.outbound);
            tokio::spawn(async move {
                handle_connection(stream, peer, events_tx, outbound).await;
                // Best-effort disconnect notification. If the
                // consumer dropped its receiver (e.g. wyoming-tap
                // doesn't care), `send` returns `SendError(Closed)`
                // and we move on — the server is not blocked.
                let _ = disconnects_tx.send(peer);
            });
        }
    }
}

/// Handle for sending Wyoming events to specific connected peers.
#[derive(Clone)]
pub struct WyomingSender {
    outbound: Arc<Mutex<HashMap<SocketAddr, mpsc::Sender<Event>>>>,
}

impl WyomingSender {
    /// Send a single event to `peer`. Returns `SendError::NotConnected`
    /// if the peer is not currently connected (or disconnected
    /// between the lookup and the send).
    pub async fn send_to(
        &self,
        peer: SocketAddr,
        event: Event,
    ) -> std::result::Result<(), SendError> {
        let tx = self
            .outbound
            .lock()
            .unwrap()
            .get(&peer)
            .cloned()
            .ok_or(SendError::NotConnected)?;
        tx.send(event).await.map_err(|_| SendError::NotConnected)
    }

    /// Frame `pcm` as `audio-start` → N `audio-chunk`s → `audio-stop`
    /// and send the sequence to `peer`.
    ///
    /// Chunk size is rounded down to a multiple of the frame size
    /// `(bits_per_sample / 8) * channels` so no audio frame is split
    /// across chunk boundaries. Trailing bytes that do not form a
    /// whole frame are dropped and logged at `debug` level. Empty
    /// PCM yields `audio-start` immediately followed by `audio-stop`.
    ///
    /// If a chunk send fails (e.g. the peer disconnects mid-stream),
    /// `audio-stop` is still sent as a best-effort attempt to reset
    /// the satellite's audio state before the error is returned.
    pub async fn send_audio(
        &self,
        peer: SocketAddr,
        pcm: &[u8],
        format: AudioFormat,
    ) -> std::result::Result<(), SendError> {
        let frame_size = (format.bits_per_sample / 8) as usize * format.channels as usize;
        let chunk_size = match AUDIO_CHUNK_BYTES.checked_div(frame_size) {
            None => AUDIO_CHUNK_BYTES, // frame_size == 0
            Some(0) => frame_size,     // frame_size > AUDIO_CHUNK_BYTES
            Some(n) => n * frame_size,
        };

        self.send_to(
            peer,
            Event {
                kind: EventKind::AudioStart,
                data: json!({
                    "rate": format.sample_rate_hz,
                    "width": format.bits_per_sample / 8,
                    "channels": format.channels,
                }),
                payload: Vec::new(),
                version: None,
            },
        )
        .await?;

        // Truncate to whole frames so no audio frame is split.
        let valid_len = if frame_size == 0 {
            pcm.len()
        } else {
            pcm.len() - (pcm.len() % frame_size)
        };
        if valid_len < pcm.len() {
            debug!(
                "dropping {} trailing bytes from {}-byte PCM for {peer} (not a whole frame)",
                pcm.len() - valid_len,
                pcm.len()
            );
        }

        let chunk_result = async {
            for chunk in pcm[..valid_len].chunks(chunk_size) {
                self.send_to(
                    peer,
                    Event {
                        kind: EventKind::AudioChunk,
                        data: json!({}),
                        payload: chunk.to_vec(),
                        version: None,
                    },
                )
                .await?;
            }
            Ok(())
        }
        .await;

        // Best-effort audio-stop so the satellite doesn't stay stuck
        // in audio-receiving state even if chunks failed.
        let stop_result = self
            .send_to(
                peer,
                Event {
                    kind: EventKind::AudioStop,
                    data: json!({}),
                    payload: Vec::new(),
                    version: None,
                },
            )
            .await;

        match chunk_result {
            Err(e) => Err(e),
            Ok(()) => stop_result,
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    tx: mpsc::Sender<IncomingEvent>,
    outbound: Arc<Mutex<HashMap<SocketAddr, mpsc::Sender<Event>>>>,
) {
    let (read_half, write_half) = stream.into_split();
    let mut reader = WyomingReader::new(read_half);

    let (outbound_tx, mut outbound_rx) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
    {
        let mut map = outbound.lock().unwrap();
        if map.contains_key(&peer) {
            warn!("rejecting duplicate connection from {peer}");
            return;
        }
        map.insert(peer, outbound_tx);
    }

    let mut writer = WyomingWriter::new(write_half);

    let read_fut = async {
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
    };

    let write_fut = async {
        while let Some(event) = outbound_rx.recv().await {
            if let Err(e) = writer.write_event(&event).await {
                warn!("Wyoming write error to {peer}: {e}");
                break;
            }
        }
    };

    tokio::select! {
        _ = read_fut => {}
        _ = write_fut => {}
    }

    outbound.lock().unwrap().remove(&peer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{WyomingReader, WyomingWriter};
    use crate::event::EventKind;
    use crate::session::AudioFormat;
    use serde_json::{Value, json};
    use std::time::Duration;
    use tokio::net::TcpStream;

    fn event(kind: EventKind) -> Event {
        Event {
            kind,
            data: Value::Null,
            payload: Vec::new(),
            version: None,
        }
    }

    /// End-to-end: bind on an OS-assigned port, connect a client,
    /// send three events, confirm the server task forwards them.
    #[tokio::test]
    async fn server_forwards_events_to_channel() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut rx, _disconnects) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();

        // Run the accept loop in the background.
        let _server_task = tokio::spawn(server.run());

        // Client: send describe, audio-chunk (with payload), audio-stop.
        let stream = TcpStream::connect(bound).await.unwrap();
        let mut writer = WyomingWriter::new(stream);
        writer
            .write_event(&event(EventKind::Describe))
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
            .write_event(&event(EventKind::AudioStop))
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
        let _server_task = tokio::spawn(server.run());

        // Open a connection, send one event so the server learns the
        // peer address from the wire (and we can confirm it matches),
        // then drop the stream to trigger EOF on the server.
        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let mut writer = WyomingWriter::new(stream);
        writer.write_event(&event(EventKind::Ping)).await.unwrap();
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

    /// Server sends a single event to a connected peer; client reads
    /// it back and asserts kind, data, and payload.
    #[tokio::test]
    async fn send_to_round_trips() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();

        let ev = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");
        assert_eq!(ev.from, client_peer);

        let sent = Event {
            kind: EventKind::Pong,
            data: json!({"ok": true}),
            payload: vec![0x01, 0x02],
            version: None,
        };
        sender.send_to(client_peer, sent.clone()).await.unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");

        assert_eq!(received.kind, EventKind::Pong);
        assert_eq!(received.data, json!({"ok": true}));
        assert_eq!(received.payload, vec![0x01, 0x02]);
    }

    /// PCM larger than AUDIO_CHUNK_BYTES is split into multiple
    /// audio-chunk events; client reads the full sequence and the
    /// concatenated payload equals the original PCM.
    #[tokio::test]
    async fn send_audio_frames_correctly() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");

        let pcm = vec![0xabu8; 5000];
        let format = AudioFormat {
            sample_rate_hz: 16000,
            bits_per_sample: 16,
            channels: 1,
        };
        sender.send_audio(client_peer, &pcm, format).await.unwrap();

        let start = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(start.kind, EventKind::AudioStart);
        assert_eq!(start.data["rate"], 16000);
        assert_eq!(start.data["width"], 2);
        assert_eq!(start.data["channels"], 1);

        let mut collected = Vec::new();
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
                .await
                .expect("timed out")
                .expect("read result")
                .expect("event");
            match ev.kind {
                EventKind::AudioChunk => {
                    collected.extend_from_slice(&ev.payload);
                }
                EventKind::AudioStop => break,
                other => panic!("unexpected event kind: {:?}", other),
            }
        }

        assert_eq!(collected, pcm);
    }

    /// PCM length is not a multiple of the chunk size; the last
    /// chunk is a remainder and every chunk size is a whole number
    /// of frames.
    #[tokio::test]
    async fn send_audio_chunk_boundaries() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");

        // frame_size = 2 bytes * 2 channels = 4 bytes
        // chunk_size = (2048 / 4) * 4 = 2048
        // 5000 bytes → 2 full chunks + remainder 904 (divisible by 4)
        let pcm = vec![0xabu8; 5000];
        let format = AudioFormat {
            sample_rate_hz: 16000,
            bits_per_sample: 16,
            channels: 2,
        };
        sender.send_audio(client_peer, &pcm, format).await.unwrap();

        let start = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(start.kind, EventKind::AudioStart);

        let mut chunk_sizes = Vec::new();
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
                .await
                .expect("timed out")
                .expect("read result")
                .expect("event");
            match ev.kind {
                EventKind::AudioChunk => {
                    chunk_sizes.push(ev.payload.len());
                }
                EventKind::AudioStop => break,
                other => panic!("unexpected event kind: {:?}", other),
            }
        }

        assert_eq!(chunk_sizes.len(), 3);
        assert_eq!(chunk_sizes[0], 2048);
        assert_eq!(chunk_sizes[1], 2048);
        assert_eq!(chunk_sizes[2], 904);
        assert_eq!(chunk_sizes.iter().sum::<usize>(), pcm.len());
    }

    /// Empty PCM sends audio-start immediately followed by audio-stop
    /// with zero chunks in between.
    #[tokio::test]
    async fn send_audio_empty_pcm() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");

        let format = AudioFormat {
            sample_rate_hz: 16000,
            bits_per_sample: 16,
            channels: 1,
        };
        sender.send_audio(client_peer, &[], format).await.unwrap();

        let start = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(start.kind, EventKind::AudioStart);

        let stop = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(stop.kind, EventKind::AudioStop);
    }

    /// Sending to a peer that never connected returns NotConnected.
    #[tokio::test]
    async fn send_to_unknown_peer() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, _events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let unknown: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let result = sender.send_to(unknown, event(EventKind::Ping)).await;
        assert_eq!(result, Err(SendError::NotConnected));
    }

    /// When frame_size exceeds AUDIO_CHUNK_BYTES, chunk_size falls
    /// back to frame_size so `.chunks()` never receives 0.
    #[tokio::test]
    async fn send_audio_large_frame_size() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");

        // frame_size = 4 * 600 = 2400 > AUDIO_CHUNK_BYTES (2048)
        let pcm = vec![0xabu8; 5000];
        let format = AudioFormat {
            sample_rate_hz: 16000,
            bits_per_sample: 32,
            channels: 600,
        };
        sender.send_audio(client_peer, &pcm, format).await.unwrap();

        let start = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(start.kind, EventKind::AudioStart);

        let mut chunk_sizes = Vec::new();
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
                .await
                .expect("timed out")
                .expect("read result")
                .expect("event");
            match ev.kind {
                EventKind::AudioChunk => {
                    chunk_sizes.push(ev.payload.len());
                }
                EventKind::AudioStop => break,
                other => panic!("unexpected event kind: {:?}", other),
            }
        }

        assert_eq!(chunk_sizes.len(), 2);
        assert_eq!(chunk_sizes[0], 2400);
        assert_eq!(chunk_sizes[1], 2400);
        assert_eq!(chunk_sizes.iter().sum::<usize>(), 4800); // 5000 - (5000 % 2400)
    }

    /// Trailing bytes that do not make up a whole frame are dropped.
    #[tokio::test]
    async fn send_audio_drops_trailing_partial_frame() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, _disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut reader = WyomingReader::new(read_half);
        let mut writer = WyomingWriter::new(write_half);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");

        // frame_size = 2 * 2 = 4; 5001 % 4 = 1 trailing byte
        let pcm = vec![0xabu8; 5001];
        let format = AudioFormat {
            sample_rate_hz: 16000,
            bits_per_sample: 16,
            channels: 2,
        };
        sender.send_audio(client_peer, &pcm, format).await.unwrap();

        let start = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
            .await
            .expect("timed out")
            .expect("read result")
            .expect("event");
        assert_eq!(start.kind, EventKind::AudioStart);

        let mut collected = 0;
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(2), reader.read_event())
                .await
                .expect("timed out")
                .expect("read result")
                .expect("event");
            match ev.kind {
                EventKind::AudioChunk => {
                    collected += ev.payload.len();
                    assert_eq!(ev.payload.len() % 4, 0, "every chunk must be whole frames");
                }
                EventKind::AudioStop => break,
                other => panic!("unexpected event kind: {:?}", other),
            }
        }

        assert_eq!(collected, 5000); // dropped the trailing byte
    }

    /// After a client disconnects, the peer is removed from the
    /// outbound map and subsequent sends return NotConnected.
    #[tokio::test]
    async fn outbound_deregistered_on_disconnect() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (server, mut events_rx, mut disconnects_rx) = WyomingServer::bind(addr).await.unwrap();
        let bound = server.listener.local_addr().unwrap();
        let sender = server.sender();
        let _server_task = tokio::spawn(server.run());

        let stream = TcpStream::connect(bound).await.unwrap();
        let client_peer = stream.local_addr().unwrap();
        let mut writer = WyomingWriter::new(stream);

        writer.write_event(&event(EventKind::Ping)).await.unwrap();

        let ev = tokio::time::timeout(Duration::from_secs(2), events_rx.recv())
            .await
            .expect("timed out")
            .expect("event");
        assert_eq!(ev.from, client_peer);

        drop(writer);

        let dropped = tokio::time::timeout(Duration::from_secs(2), disconnects_rx.recv())
            .await
            .expect("timed out")
            .expect("disconnect");
        assert_eq!(dropped, client_peer);

        let result = sender.send_to(client_peer, event(EventKind::Ping)).await;
        assert_eq!(result, Err(SendError::NotConnected));
    }
}

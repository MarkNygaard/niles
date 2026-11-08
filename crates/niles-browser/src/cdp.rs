//! The websocket connection to Chromium's DevTools: calls matched by id,
//! events broadcast, close handling.
//!
//! No log line here carries parameters, results or event params.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Result};

/// How long one call waits for its answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// An event the browser broadcast, maybe on a session.
#[derive(Debug, Clone)]
pub(crate) struct Event {
    pub(crate) session: Option<String>,
    pub(crate) method: String,
    pub(crate) params: Value,
}

type Pending = Mutex<HashMap<u64, (&'static str, oneshot::Sender<Result<Value>>)>>;

/// One websocket to DevTools, shared by every session on it.
pub(crate) struct Connection {
    writer: mpsc::Sender<String>,
    pending: Arc<Pending>,
    next: AtomicU64,
    /// Set to `None` when the stream ends, so every subscriber's
    /// receiver closes.
    events: Arc<Mutex<Option<broadcast::Sender<Event>>>>,
    closed: Arc<AtomicBool>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

impl Connection {
    /// Spawns the reader and writer over `ws`.
    pub(crate) fn open(ws: WebSocketStream<TcpStream>) -> Arc<Connection> {
        let (mut sink, mut stream) = ws.split();
        let (writer_tx, mut writer_rx) = mpsc::channel::<String>(64);
        let (events_tx, _) = broadcast::channel(1024);
        let events = Arc::new(Mutex::new(Some(events_tx)));
        let pending = Arc::new(Mutex::new(HashMap::new()));

        let reader_pending = Arc::clone(&pending);
        let reader_events = Arc::clone(&events);
        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);

        let reader_task = tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                let Ok(Message::Text(text)) = frame else {
                    // A close, a stream error or a stray binary frame: on
                    // an error the stream is done; anything else carries
                    // on, and none of it is content.
                    if frame.is_err() {
                        break;
                    }
                    continue;
                };
                dispatch(&text, &reader_pending, &reader_events);
            }
            reader_closed.store(true, Ordering::SeqCst);
            for (_, (_, reply)) in reader_pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
            {
                let _ = reply.send(Err(Error::Closed));
            }
            // The last sender drops: every subscriber's receiver closes.
            drop(
                reader_events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take(),
            );
        });

        let writer_task = tokio::spawn(async move {
            while let Some(text) = writer_rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });

        Arc::new(Connection {
            writer: writer_tx,
            pending,
            next: AtomicU64::new(1),
            events,
            closed,
            reader_task,
            writer_task,
        })
    }

    /// One CDP call, matched to its answer by id. `session` is the flat
    /// session id when the call is on an attached target.
    pub(crate) async fn call(
        &self,
        method: &'static str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, (method, reply_tx));

        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_string());
        }
        // A closed writer ends the call like a closed stream would.
        if self.writer.send(message.to_string()).await.is_err() {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(Error::Closed);
        }

        match tokio::time::timeout(CALL_TIMEOUT, reply_rx).await {
            Ok(answer) => answer.unwrap_or(Err(Error::Closed)),
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err(Error::Timeout { method })
            }
        }
    }

    /// Subscribes to every event the browser broadcasts. A closed
    /// connection gives a receiver that ends at once.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
        match &*self.events.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(sender) => sender.subscribe(),
            None => {
                let (sender, receiver) = broadcast::channel(1);
                drop(sender);
                receiver
            }
        }
    }

    /// Whether the stream to the browser has ended.
    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.writer_task.abort();
    }
}

/// Routes one text frame: answers to the pending call, events to every
/// subscriber. Malformed frames are dropped with a log that names no
/// content.
fn dispatch(text: &str, pending: &Pending, events: &Arc<Mutex<Option<broadcast::Sender<Event>>>>) {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        tracing::debug!("[browser] dropped a malformed DevTools frame");
        return;
    };
    if let Some(id) = frame.get("id").and_then(Value::as_u64) {
        let Some((method, reply)) = pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        else {
            return;
        };
        let answer = match frame.get("error") {
            Some(error) => {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                // CDP's `data` may carry page content; never forward it.
                Err(Error::Protocol {
                    method,
                    reason: format!("{message} ({code})"),
                })
            }
            None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = reply.send(answer);
        return;
    }
    if let Some(method) = frame.get("method").and_then(Value::as_str) {
        let guard = events.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(Event {
                session: frame
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                method: method.to_string(),
                params: frame.get("params").cloned().unwrap_or(Value::Null),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    /// Starts a DevTools stand-in answering by a method-keyed script.
    async fn stand_in() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept ws");
            let mut one: Option<u64> = None;
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: Value = serde_json::from_str(&text).expect("json");
                let id = frame["id"].as_u64().expect("id");
                match frame["method"].as_str().expect("method") {
                    // Remember the id, answer once "two" arrives too.
                    "one" => one = Some(id),
                    // Answer the second call first: matching is by id,
                    // not by order.
                    "two" => {
                        let one = one.expect("one called before two");
                        for (answered, n) in [(id, 2), (one, 1)] {
                            ws.send(Message::Text(
                                json!({"id": answered, "result": {"n": n}})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .expect("send");
                        }
                    }
                    "err" => {
                        ws.send(Message::Text(
                            json!({"id": id, "error": {"code": -32000, "message": "No such thing"}})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .expect("send");
                    }
                    // Send an event on the caller's session, then answer.
                    "event" => {
                        ws.send(Message::Text(
                            json!({
                                "method": "Page.frameStartedLoading",
                                "params": {"frameId": "f1"},
                                "sessionId": frame["sessionId"],
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send");
                        ws.send(Message::Text(
                            json!({"id": id, "result": {}}).to_string().into(),
                        ))
                        .await
                        .expect("send");
                    }
                    // Drop the connection without answering.
                    "die" => break,
                    _ => {}
                }
            }
        });
        addr
    }

    async fn connect() -> Arc<Connection> {
        let addr = stand_in().await;
        let stream = TcpStream::connect(addr).await.expect("tcp");
        let (ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}"), stream)
            .await
            .expect("handshake");
        Connection::open(ws)
    }

    #[tokio::test]
    async fn responses_are_matched_by_id_under_concurrent_calls() {
        let conn = connect().await;
        let first = tokio::spawn({
            let conn = Arc::clone(&conn);
            async move { conn.call("one", Value::Null, None).await }
        });
        // Let "one" reach the server before "two" starts.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let second = conn.call("two", Value::Null, None).await;
        let first = first.await.expect("join");
        assert_eq!(first.expect("first"), json!({"n": 1}));
        assert_eq!(second.expect("second"), json!({"n": 2}));
    }

    #[tokio::test]
    async fn a_cdp_error_becomes_protocol_with_the_method_and_message() {
        let conn = connect().await;
        let err = conn.call("err", Value::Null, None).await.unwrap_err();
        match err {
            Error::Protocol { method, reason } => {
                assert_eq!(method, "err");
                assert!(reason.contains("No such thing"), "{reason}");
                assert!(reason.contains("-32000"), "{reason}");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_close_fails_pending_calls_and_sets_closed() {
        let conn = connect().await;
        let mut rx = conn.subscribe();
        let pending = tokio::spawn({
            let conn = Arc::clone(&conn);
            async move { conn.call("die", Value::Null, None).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(conn.closed());
        match pending.await.expect("join") {
            Err(Error::Closed) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
        // The broadcast sender is dropped: the stream ends.
        assert!(rx.recv().await.is_err());
    }

    #[tokio::test]
    async fn events_carry_their_session_id() {
        let conn = connect().await;
        let mut rx = conn.subscribe();
        let answer = conn.call("event", Value::Null, Some("s7")).await;
        assert_eq!(answer.expect("answer"), json!({}));
        let event = rx.recv().await.expect("event");
        assert_eq!(event.session.as_deref(), Some("s7"));
        assert_eq!(event.method, "Page.frameStartedLoading");
        assert_eq!(event.params, json!({"frameId": "f1"}));
    }
}

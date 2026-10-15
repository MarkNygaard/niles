//! Wyoming frame codec.
//!
//! Reads/writes Wyoming events over any `AsyncRead` / `AsyncWrite`.
//! The wire format is: a JSON header on a single line terminated by
//! `\n`, optionally followed by exactly `payload_length` bytes of
//! raw binary data (no further delimiter).

use crate::error::{Error, Result};
use crate::event::{Event, EventKind, WireHeader};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Hard cap on the JSON header size we'll accept. Wyoming headers
/// are normally tiny (a few hundred bytes); 64 KB is comfortably
/// over any realistic value while still preventing a malicious
/// peer from making us buffer a gigabyte.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Hard cap on the binary payload per frame. Audio chunks are
/// typically a few KB; 1 MB is generous headroom.
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Async reader for Wyoming frames. Wraps any `AsyncRead` in a
/// `BufReader` so the header line can be scanned in place.
pub struct WyomingReader<R> {
    inner: BufReader<R>,
    line: Vec<u8>,
}

impl<R: AsyncRead + Unpin> WyomingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner: BufReader::new(inner),
            line: Vec::new(),
        }
    }

    /// Read one event from the stream. Returns `Ok(None)` on clean
    /// EOF (peer closed); `Err` on malformed or truncated input.
    pub async fn read_event(&mut self) -> Result<Option<Event>> {
        if !self.read_header_line().await? {
            return Ok(None); // clean EOF
        }
        // Strip the trailing \n, and the \r of a CRLF sender. Any other
        // trailing whitespace goes too, which JSON never minds.
        let trimmed = self.line.trim_ascii_end();
        if trimmed.is_empty() {
            return Err(Error::Frame {
                reason: "received empty header line".into(),
            });
        }

        let header: WireHeader = serde_json::from_slice(trimmed)?;

        let payload_length = header.payload_length.unwrap_or(0);
        if payload_length > MAX_PAYLOAD_BYTES {
            return Err(Error::Frame {
                reason: format!(
                    "payload_length {payload_length} exceeds {MAX_PAYLOAD_BYTES}-byte limit"
                ),
            });
        }

        let mut payload = vec![0u8; payload_length];
        if payload_length > 0 {
            self.inner.read_exact(&mut payload).await?;
        }

        // Normalize a missing or explicit-null `data` field to an
        // empty JSON object so consumers see one canonical shape.
        let data = if header.data.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            header.data
        };

        Ok(Some(Event {
            kind: EventKind::from(header.type_.as_str()),
            data,
            payload,
            version: header.version,
        }))
    }

    /// Read the header line, newline included, into `self.line`.
    /// Returns false on EOF before the first byte.
    ///
    /// The limit is enforced as bytes arrive. `read_line` only returns
    /// at a newline or EOF, so checking its result afterwards let a peer
    /// that never sent a newline make us hold everything it sent.
    async fn read_header_line(&mut self) -> Result<bool> {
        self.line.clear();
        loop {
            let available = self.inner.fill_buf().await?;
            if available.is_empty() {
                // EOF. Partway through a line, hand over what there is,
                // as `read_line` did; the parse decides whether it was
                // a whole header.
                return Ok(!self.line.is_empty());
            }
            let (take, complete) = match available.iter().position(|&b| b == b'\n') {
                Some(newline) => (newline + 1, true),
                None => (available.len(), false),
            };
            if self.line.len() + take > MAX_HEADER_BYTES {
                return Err(Error::Frame {
                    reason: format!("header exceeds {MAX_HEADER_BYTES}-byte limit"),
                });
            }
            self.line.extend_from_slice(&available[..take]);
            self.inner.consume(take);
            if complete {
                return Ok(true);
            }
        }
    }
}

/// Async writer for Wyoming frames.
pub struct WyomingWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> WyomingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    /// Serialize and write one event. The payload is sent
    /// immediately after the header newline.
    pub async fn write_event(&mut self, event: &Event) -> Result<()> {
        let header = WireHeader {
            type_: event.kind.as_wire_str().to_string(),
            data: if event.data.is_null() {
                Value::Object(serde_json::Map::new())
            } else {
                event.data.clone()
            },
            payload_length: (!event.payload.is_empty()).then_some(event.payload.len()),
            version: event.version.clone(),
        };
        let mut line = serde_json::to_vec(&header)?;
        line.push(b'\n');
        self.inner.write_all(&line).await?;
        if !event.payload.is_empty() {
            self.inner.write_all(&event.payload).await?;
        }
        self.inner.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    /// Send `bytes` and keep the connection open, the way a peer that
    /// never finishes its header would, then read one event.
    async fn read_from_a_peer_that_stays_open(bytes: Vec<u8>) -> Result<Option<Event>> {
        use tokio::io::AsyncWriteExt;
        let (mut writer, stream) = tokio::io::duplex(4096);
        let producer = tokio::spawn(async move {
            writer.write_all(&bytes).await.unwrap();
            writer
        });
        let mut reader = WyomingReader::new(stream);
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(250), reader.read_event())
                .await
                .expect("the reader waited for a newline instead of refusing the header");
        producer.abort();
        result
    }

    /// A `describe` header line of exactly `len` bytes, newline included.
    fn header_line_of(len: usize) -> Vec<u8> {
        let prefix = br#"{"type":"describe","data":{"pad":""#;
        let suffix = b"\"}}\n";
        let mut line = prefix.to_vec();
        line.resize(len - suffix.len(), b'x');
        line.extend_from_slice(suffix);
        assert_eq!(line.len(), len);
        line
    }

    // From an outside review: the 64 KiB cap was checked only after
    // `read_line` returned, which is at a newline or EOF, so a peer
    // that sent neither was buffered without limit.
    #[tokio::test]
    async fn an_unterminated_header_is_refused_without_waiting_for_a_newline() {
        let result = read_from_a_peer_that_stays_open(vec![b'x'; MAX_HEADER_BYTES * 4]).await;
        assert!(
            matches!(result, Err(Error::Frame { .. })),
            "oversize unterminated header was not rejected; result={result:?}"
        );
    }

    #[tokio::test]
    async fn an_unterminated_header_is_refused_at_the_first_byte_over_the_limit() {
        let result = read_from_a_peer_that_stays_open(vec![b'x'; MAX_HEADER_BYTES + 1]).await;
        assert!(
            matches!(result, Err(Error::Frame { .. })),
            "result={result:?}"
        );
    }

    #[tokio::test]
    async fn a_header_exactly_at_the_limit_is_read_and_the_next_event_after_it() {
        let mut raw = header_line_of(MAX_HEADER_BYTES);
        raw.extend_from_slice(b"{\"type\":\"ping\"}\n");
        let mut reader = WyomingReader::new(Cursor::new(raw));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.kind, EventKind::Describe);
        assert_eq!(
            reader.read_event().await.unwrap().expect("event").kind,
            EventKind::Ping
        );
    }

    #[tokio::test]
    async fn a_header_one_byte_over_the_limit_is_refused() {
        let raw = header_line_of(MAX_HEADER_BYTES + 1);
        let mut reader = WyomingReader::new(Cursor::new(raw));
        let result = reader.read_event().await;
        assert!(
            matches!(result, Err(Error::Frame { .. })),
            "result={result:?}"
        );
    }

    #[tokio::test]
    async fn reads_a_simple_header_only_event() {
        let raw = b"{\"type\":\"describe\"}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.kind, EventKind::Describe);
        assert!(event.payload.is_empty());
    }

    #[tokio::test]
    async fn reads_event_with_data() {
        let raw = b"{\"type\":\"transcript\",\"data\":{\"text\":\"hello\"}}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.kind, EventKind::Transcript);
        assert_eq!(event.data, json!({"text": "hello"}));
    }

    #[tokio::test]
    async fn reads_event_with_binary_payload() {
        let mut raw: Vec<u8> =
            br#"{"type":"audio-chunk","data":{"rate":16000},"payload_length":4}"#.to_vec();
        raw.push(b'\n');
        raw.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let mut reader = WyomingReader::new(Cursor::new(raw));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.kind, EventKind::AudioChunk);
        assert_eq!(event.payload, vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(event.data["rate"], 16000);
    }

    #[tokio::test]
    async fn reads_multiple_events_from_one_stream() {
        let mut raw: Vec<u8> =
            br#"{"type":"audio-start","data":{"rate":16000,"width":2,"channels":1}}"#.to_vec();
        raw.push(b'\n');
        raw.extend_from_slice(br#"{"type":"audio-chunk","payload_length":2}"#);
        raw.push(b'\n');
        raw.extend_from_slice(&[0x00, 0x01]);
        raw.extend_from_slice(br#"{"type":"audio-stop"}"#);
        raw.push(b'\n');
        let mut reader = WyomingReader::new(Cursor::new(raw));

        let e1 = reader.read_event().await.unwrap().expect("e1");
        assert_eq!(e1.kind, EventKind::AudioStart);
        let e2 = reader.read_event().await.unwrap().expect("e2");
        assert_eq!(e2.kind, EventKind::AudioChunk);
        assert_eq!(e2.payload, vec![0x00, 0x01]);
        let e3 = reader.read_event().await.unwrap().expect("e3");
        assert_eq!(e3.kind, EventKind::AudioStop);
        assert!(reader.read_event().await.unwrap().is_none(), "EOF expected");
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let mut reader = WyomingReader::new(Cursor::new(Vec::<u8>::new()));
        assert!(reader.read_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_json_errors() {
        let raw = b"not json\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        assert!(reader.read_event().await.is_err());
    }

    #[tokio::test]
    async fn truncated_payload_errors() {
        let mut raw: Vec<u8> = br#"{"type":"audio-chunk","payload_length":10}"#.to_vec();
        raw.push(b'\n');
        raw.extend_from_slice(&[0xff, 0xff]); // only 2 of 10 promised bytes
        let mut reader = WyomingReader::new(Cursor::new(raw));
        assert!(reader.read_event().await.is_err());
    }

    #[tokio::test]
    async fn handles_crlf_line_endings() {
        let raw = b"{\"type\":\"ping\"}\r\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.kind, EventKind::Ping);
    }

    #[tokio::test]
    async fn missing_data_field_normalizes_to_empty_object() {
        let raw = b"{\"type\":\"describe\"}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.data, json!({}));
    }

    #[tokio::test]
    async fn explicit_null_data_normalizes_to_empty_object() {
        let raw = b"{\"type\":\"describe\",\"data\":null}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(event.data, json!({}));
    }

    #[tokio::test]
    async fn reuses_buffer_across_reads() {
        // Two events back-to-back, then EOF. The reader's internal
        // line buffer is reused; this just confirms the round-trip
        // still works (allocation behavior isn't directly testable).
        let raw = b"{\"type\":\"ping\"}\n{\"type\":\"pong\"}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        assert_eq!(
            reader.read_event().await.unwrap().expect("e1").kind,
            EventKind::Ping
        );
        assert_eq!(
            reader.read_event().await.unwrap().expect("e2").kind,
            EventKind::Pong
        );
        assert!(reader.read_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_event_type_parses_as_other() {
        let raw = b"{\"type\":\"some-future-event\"}\n";
        let mut reader = WyomingReader::new(Cursor::new(&raw[..]));
        let event = reader.read_event().await.unwrap().expect("event");
        assert_eq!(
            event.kind,
            EventKind::Other("some-future-event".to_string())
        );
    }

    #[tokio::test]
    async fn writer_round_trips_header_only_event() {
        let event = Event {
            kind: EventKind::Describe,
            data: Value::Null,
            payload: Vec::new(),
            version: None,
        };
        let mut buf = Vec::new();
        WyomingWriter::new(&mut buf)
            .write_event(&event)
            .await
            .unwrap();

        let mut reader = WyomingReader::new(Cursor::new(buf));
        let parsed = reader.read_event().await.unwrap().expect("event");
        assert_eq!(parsed.kind, EventKind::Describe);
        assert!(parsed.payload.is_empty());
    }

    #[tokio::test]
    async fn writer_round_trips_event_with_payload() {
        let event = Event {
            kind: EventKind::AudioChunk,
            data: json!({"rate": 16000, "width": 2, "channels": 1}),
            payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
            version: None,
        };
        let mut buf = Vec::new();
        WyomingWriter::new(&mut buf)
            .write_event(&event)
            .await
            .unwrap();

        let mut reader = WyomingReader::new(Cursor::new(buf));
        let parsed = reader.read_event().await.unwrap().expect("event");
        assert_eq!(parsed.kind, EventKind::AudioChunk);
        assert_eq!(parsed.payload, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(parsed.data["rate"], 16000);
    }
}

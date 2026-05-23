//! Per-peer audio-session accumulator.
//!
//! Wyoming satellites stream voice utterances as:
//!
//! ```text
//! audio-start  { rate, width, channels }
//! audio-chunk  + binary payload         (N times)
//! audio-stop
//! ```
//!
//! [`SessionTracker`] turns those event streams into completed
//! [`AudioSession`] objects — one per `audio-start` ... `audio-stop`
//! cycle — so downstream code (STT) gets a single buffered blob
//! instead of a stream of partial chunks.
//!
//! State is keyed by peer `SocketAddr` so multiple satellites can
//! talk to the same server without their audio interleaving.

use crate::event::{Event, EventKind};
use crate::server::IncomingEvent;
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::{debug, warn};

/// Cap on how much PCM we'll buffer per in-flight session before
/// giving up. 5 MB ≈ 2.5 minutes at 16 kHz mono 16-bit — well past
/// any sane voice command. A satellite that streams that long
/// without an `audio-stop` is either broken or attacking us.
const MAX_SESSION_PCM_BYTES: usize = 5 * 1024 * 1024;

/// PCM format declared in the `audio-start` event.
///
/// `#[non_exhaustive]` so callers in other crates can't bypass
/// [`parse_audio_format`]'s validation by constructing one directly
/// with zeroed or out-of-range fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    /// Bits per sample, derived from Wyoming's `width` (bytes).
    pub bits_per_sample: u16,
    pub channels: u16,
}

/// One completed audio utterance: the format declared at start, plus
/// every PCM byte received between `audio-start` and `audio-stop`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AudioSession {
    pub from: SocketAddr,
    pub format: AudioFormat,
    pub pcm: Vec<u8>,
}

#[derive(Debug)]
struct InFlight {
    format: AudioFormat,
    pcm: Vec<u8>,
    /// True once we've decided to discard this session (e.g.
    /// oversize, malformed). Subsequent chunks are ignored until
    /// `audio-stop` resets the slot.
    poisoned: bool,
}

/// Driver state. One `SessionTracker` per Wyoming server is enough —
/// keyed internally by peer address so concurrent connections don't
/// clobber each other.
#[derive(Debug, Default)]
pub struct SessionTracker {
    in_flight: HashMap<SocketAddr, InFlight>,
}

impl SessionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one inbound event. Returns `Some(AudioSession)` when an
    /// `audio-stop` closes a valid in-flight session. Returns `None`
    /// for every other event (and for closed sessions that were
    /// poisoned along the way).
    pub fn feed(&mut self, incoming: IncomingEvent) -> Option<AudioSession> {
        let IncomingEvent { from, event } = incoming;
        match event.kind {
            EventKind::AudioStart => {
                if self.in_flight.contains_key(&from) {
                    warn!("{from}: audio-start while a session was already open — restarting");
                }
                match parse_audio_format(&event) {
                    Ok(format) => {
                        self.in_flight.insert(
                            from,
                            InFlight {
                                format,
                                pcm: Vec::new(),
                                poisoned: false,
                            },
                        );
                    }
                    Err(reason) => {
                        warn!("{from}: discarding session — bad audio-start: {reason}");
                        // Insert a poisoned slot so subsequent
                        // chunks aren't mistaken for a fresh session.
                        self.in_flight.insert(
                            from,
                            InFlight {
                                format: AudioFormat {
                                    sample_rate_hz: 0,
                                    bits_per_sample: 0,
                                    channels: 0,
                                },
                                pcm: Vec::new(),
                                poisoned: true,
                            },
                        );
                    }
                }
                None
            }
            EventKind::AudioChunk => {
                let slot = self.in_flight.get_mut(&from)?;
                if slot.poisoned {
                    return None;
                }
                if slot.pcm.len() + event.payload.len() > MAX_SESSION_PCM_BYTES {
                    warn!(
                        "{from}: session exceeds {} bytes — poisoning",
                        MAX_SESSION_PCM_BYTES
                    );
                    slot.poisoned = true;
                    slot.pcm = Vec::new(); // free what we've accumulated
                    return None;
                }
                slot.pcm.extend_from_slice(&event.payload);
                None
            }
            EventKind::AudioStop => {
                let slot = self.in_flight.remove(&from)?;
                if slot.poisoned {
                    debug!("{from}: audio-stop closes a poisoned session — dropped");
                    return None;
                }
                Some(AudioSession {
                    from,
                    format: slot.format,
                    pcm: slot.pcm,
                })
            }
            // Voice-started / voice-stopped / ping etc. don't gate
            // sessions in v0.1 — VAD events from the satellite are
            // informational while we send the whole utterance up.
            _ => None,
        }
    }

    /// Drop any in-flight session for `peer`. Call this when a
    /// connection closes so a half-buffered utterance doesn't sit
    /// in memory waiting for an `audio-stop` that will never come.
    pub fn drop_peer(&mut self, peer: SocketAddr) {
        if self.in_flight.remove(&peer).is_some() {
            debug!("{peer}: dropping in-flight session on disconnect");
        }
    }

    #[cfg(test)]
    fn open_session_count(&self) -> usize {
        self.in_flight.len()
    }
}

/// Pull `rate` / `width` / `channels` out of an `audio-start` event's
/// `data` field. Returns a human-readable reason on the way out so
/// the caller can log it.
fn parse_audio_format(event: &Event) -> std::result::Result<AudioFormat, String> {
    let obj = event
        .data
        .as_object()
        .ok_or_else(|| "audio-start data is not an object".to_string())?;
    let rate = obj
        .get("rate")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing or non-integer `rate`".to_string())?;
    let width = obj
        .get("width")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing or non-integer `width`".to_string())?;
    let channels = obj
        .get("channels")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing or non-integer `channels`".to_string())?;
    if !(1..=4).contains(&width) {
        return Err(format!("width {width} out of supported range 1..=4"));
    }
    if rate == 0 || channels == 0 {
        return Err(format!("invalid rate={rate} channels={channels}"));
    }
    Ok(AudioFormat {
        sample_rate_hz: u32::try_from(rate).map_err(|_| format!("rate {rate} overflows u32"))?,
        bits_per_sample: u16::try_from(width * 8)
            .map_err(|_| format!("bits_per_sample for width {width} overflows u16"))?,
        channels: u16::try_from(channels)
            .map_err(|_| format!("channels {channels} overflows u16"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;
    use serde_json::Value;
    use serde_json::json;

    fn peer() -> SocketAddr {
        "127.0.0.1:50000".parse().unwrap()
    }
    fn peer_b() -> SocketAddr {
        "127.0.0.1:50001".parse().unwrap()
    }

    fn ev(kind: EventKind, data: Value, payload: Vec<u8>) -> IncomingEvent {
        IncomingEvent {
            from: peer(),
            event: Event {
                kind,
                data,
                payload,
                version: None,
            },
        }
    }

    fn ev_from(from: SocketAddr, kind: EventKind, data: Value, payload: Vec<u8>) -> IncomingEvent {
        IncomingEvent {
            from,
            event: Event {
                kind,
                data,
                payload,
                version: None,
            },
        }
    }

    fn good_start() -> Value {
        json!({"rate": 16000, "width": 2, "channels": 1})
    }

    #[test]
    fn full_cycle_emits_a_complete_session() {
        let mut t = SessionTracker::new();
        assert!(
            t.feed(ev(EventKind::AudioStart, good_start(), vec![]))
                .is_none()
        );
        assert!(
            t.feed(ev(EventKind::AudioChunk, json!({}), vec![1, 2, 3]))
                .is_none()
        );
        assert!(
            t.feed(ev(EventKind::AudioChunk, json!({}), vec![4, 5]))
                .is_none()
        );
        let session = t
            .feed(ev(EventKind::AudioStop, json!({}), vec![]))
            .expect("session emitted");
        assert_eq!(session.from, peer());
        assert_eq!(session.pcm, vec![1, 2, 3, 4, 5]);
        assert_eq!(session.format.sample_rate_hz, 16_000);
        assert_eq!(session.format.bits_per_sample, 16);
        assert_eq!(session.format.channels, 1);
        assert_eq!(t.open_session_count(), 0);
    }

    #[test]
    fn chunks_without_start_are_dropped() {
        let mut t = SessionTracker::new();
        assert!(
            t.feed(ev(EventKind::AudioChunk, json!({}), vec![1, 2, 3]))
                .is_none()
        );
        assert!(
            t.feed(ev(EventKind::AudioStop, json!({}), vec![]))
                .is_none()
        );
        assert_eq!(t.open_session_count(), 0);
    }

    #[test]
    fn second_start_restarts_the_session_buffer() {
        let mut t = SessionTracker::new();
        t.feed(ev(EventKind::AudioStart, good_start(), vec![]));
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![1, 2, 3]));
        // Second start clears the buffer.
        t.feed(ev(
            EventKind::AudioStart,
            json!({"rate": 8000, "width": 2, "channels": 1}),
            vec![],
        ));
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![9, 9]));
        let session = t
            .feed(ev(EventKind::AudioStop, json!({}), vec![]))
            .expect("session emitted");
        assert_eq!(session.pcm, vec![9, 9]);
        assert_eq!(session.format.sample_rate_hz, 8000);
    }

    #[test]
    fn malformed_audio_start_poisons_the_slot() {
        let mut t = SessionTracker::new();
        // Missing `rate` field.
        t.feed(ev(
            EventKind::AudioStart,
            json!({"width": 2, "channels": 1}),
            vec![],
        ));
        // Chunks are dropped...
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![1, 2, 3]));
        // ...and audio-stop yields nothing.
        assert!(
            t.feed(ev(EventKind::AudioStop, json!({}), vec![]))
                .is_none()
        );
        assert_eq!(t.open_session_count(), 0);
    }

    #[test]
    fn oversize_session_is_poisoned_and_freed() {
        let mut t = SessionTracker::new();
        t.feed(ev(EventKind::AudioStart, good_start(), vec![]));
        // First chunk: within cap.
        t.feed(ev(
            EventKind::AudioChunk,
            json!({}),
            vec![0u8; MAX_SESSION_PCM_BYTES],
        ));
        // Second chunk would push past the cap → poison.
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![0u8; 1]));
        // Subsequent chunks are dropped, audio-stop yields nothing.
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![0u8; 1]));
        assert!(
            t.feed(ev(EventKind::AudioStop, json!({}), vec![]))
                .is_none()
        );
        // ...and the slot is removed so we're not leaking the
        // (now-empty) poisoned entry past stop.
        assert_eq!(t.open_session_count(), 0);
    }

    #[test]
    fn drop_peer_clears_in_flight() {
        let mut t = SessionTracker::new();
        t.feed(ev(EventKind::AudioStart, good_start(), vec![]));
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![1, 2, 3]));
        assert_eq!(t.open_session_count(), 1);
        t.drop_peer(peer());
        assert_eq!(t.open_session_count(), 0);
        // Late audio-stop after disconnect is a no-op.
        assert!(
            t.feed(ev(EventKind::AudioStop, json!({}), vec![]))
                .is_none()
        );
    }

    #[test]
    fn two_peers_do_not_clobber_each_other() {
        let mut t = SessionTracker::new();
        t.feed(ev_from(peer(), EventKind::AudioStart, good_start(), vec![]));
        t.feed(ev_from(
            peer_b(),
            EventKind::AudioStart,
            good_start(),
            vec![],
        ));
        t.feed(ev_from(
            peer(),
            EventKind::AudioChunk,
            json!({}),
            vec![1, 1],
        ));
        t.feed(ev_from(
            peer_b(),
            EventKind::AudioChunk,
            json!({}),
            vec![2, 2],
        ));

        let sa = t
            .feed(ev_from(peer(), EventKind::AudioStop, json!({}), vec![]))
            .unwrap();
        assert_eq!(sa.from, peer());
        assert_eq!(sa.pcm, vec![1, 1]);

        let sb = t
            .feed(ev_from(peer_b(), EventKind::AudioStop, json!({}), vec![]))
            .unwrap();
        assert_eq!(sb.from, peer_b());
        assert_eq!(sb.pcm, vec![2, 2]);
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut t = SessionTracker::new();
        t.feed(ev(EventKind::AudioStart, good_start(), vec![]));
        // VAD + ping should not affect the buffer or close the session.
        assert!(
            t.feed(ev(EventKind::VoiceStarted, json!({}), vec![]))
                .is_none()
        );
        assert!(
            t.feed(ev(EventKind::VoiceStopped, json!({}), vec![]))
                .is_none()
        );
        assert!(t.feed(ev(EventKind::Ping, json!({}), vec![])).is_none());
        t.feed(ev(EventKind::AudioChunk, json!({}), vec![7]));
        let session = t.feed(ev(EventKind::AudioStop, json!({}), vec![])).unwrap();
        assert_eq!(session.pcm, vec![7]);
    }
}

//! Wyoming event types.
//!
//! Wyoming events are JSON objects with a required `type` field and
//! optional `data` (any JSON), `payload_length` (bytes of binary
//! payload that follow the header newline), and `version`.
//!
//! Niles models events as a single `Event` struct with `kind`, raw
//! `data` (untyped JSON until consumers care), and an in-memory
//! `payload`. The `EventKind` enum names the events we know about so
//! downstream code can `match` on them ergonomically; unknown event
//! types still parse and carry their original `type` string.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A parsed Wyoming event with its (optional) binary payload bytes
/// resolved in memory. Payload size is bounded by the sender; for
/// audio-chunk events it's typically a few KB at most.
#[derive(Debug, Clone)]
pub struct Event {
    pub kind: EventKind,
    /// Raw `data` field as JSON. Normalized to an empty object
    /// (never `Value::Null`) by the reader when the wire header omits
    /// `data` or sends an explicit `null`.
    pub data: Value,
    /// Binary payload bytes that followed the header, if any.
    pub payload: Vec<u8>,
    /// Optional `version` field from the header.
    pub version: Option<String>,
}

/// Wyoming event types we know about, plus a fallback for forward
/// compatibility. Documented event names from
/// <https://github.com/rhasspy/wyoming>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// Hello-style introduction; recipient should reply with `Info`.
    Describe,
    /// Capability announcement: ASR/TTS/intent/wake-word backends.
    Info,
    /// Connection ping; recipient should reply with a `Pong`-style ack.
    Ping,
    Pong,
    /// Start of an audio stream.
    AudioStart,
    /// A chunk of audio (binary payload).
    AudioChunk,
    /// End of an audio stream.
    AudioStop,
    /// Voice activity detected (start of speech in the audio stream).
    VoiceStarted,
    /// Voice activity ended (silence after speech).
    VoiceStopped,
    /// STT transcript event.
    Transcript,
    /// TTS request — "say this text".
    Synthesize,
    /// Recognized intent (name + slot values).
    Intent,
    /// Wake-word detection event.
    Detect,
    /// A wake-word was detected.
    Detection,
    /// Run-pipeline request from a satellite.
    RunPipeline,
    /// Any event with a type string we don't have a variant for.
    Other(String),
}

impl EventKind {
    /// The wire-format string for this event kind.
    pub fn as_wire_str(&self) -> &str {
        match self {
            Self::Describe => "describe",
            Self::Info => "info",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::AudioStart => "audio-start",
            Self::AudioChunk => "audio-chunk",
            Self::AudioStop => "audio-stop",
            Self::VoiceStarted => "voice-started",
            Self::VoiceStopped => "voice-stopped",
            Self::Transcript => "transcript",
            Self::Synthesize => "synthesize",
            Self::Intent => "intent",
            Self::Detect => "detect",
            Self::Detection => "detection",
            Self::RunPipeline => "run-pipeline",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<&str> for EventKind {
    fn from(s: &str) -> Self {
        match s {
            "describe" => Self::Describe,
            "info" => Self::Info,
            "ping" => Self::Ping,
            "pong" => Self::Pong,
            "audio-start" => Self::AudioStart,
            "audio-chunk" => Self::AudioChunk,
            "audio-stop" => Self::AudioStop,
            "voice-started" => Self::VoiceStarted,
            "voice-stopped" => Self::VoiceStopped,
            "transcript" => Self::Transcript,
            "synthesize" => Self::Synthesize,
            "intent" => Self::Intent,
            "detect" => Self::Detect,
            "detection" => Self::Detection,
            "run-pipeline" => Self::RunPipeline,
            other => Self::Other(other.to_string()),
        }
    }
}

/// On-the-wire JSON header. Used internally by the codec; consumers
/// see [`Event`] (header + resolved payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WireHeader {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default, skip_serializing_if = "is_null_or_empty")]
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn is_null_or_empty(v: &Value) -> bool {
    v.is_null() || v.as_object().is_some_and(|o| o.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_event_kinds_round_trip() {
        for s in [
            "describe",
            "info",
            "ping",
            "pong",
            "audio-start",
            "audio-chunk",
            "audio-stop",
            "voice-started",
            "voice-stopped",
            "transcript",
            "synthesize",
            "intent",
            "detect",
            "detection",
            "run-pipeline",
        ] {
            let kind = EventKind::from(s);
            assert_eq!(kind.as_wire_str(), s, "round-trip failed for {s}");
            assert!(!matches!(kind, EventKind::Other(_)), "{s} got Other(_)");
        }
    }

    #[test]
    fn unknown_event_kind_uses_other() {
        let kind = EventKind::from("custom-event");
        assert!(matches!(kind, EventKind::Other(ref s) if s == "custom-event"));
        assert_eq!(kind.as_wire_str(), "custom-event");
    }
}

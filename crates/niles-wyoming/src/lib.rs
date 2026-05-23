//! niles-wyoming — Wyoming protocol server for satellite audio.
//!
//! The Wyoming protocol is a line-delimited JSON framing over TCP,
//! with optional binary payload frames following a header. Used by
//! Home Assistant's voice satellites and ESPHome's voice_assistant
//! component to stream audio and control events between a satellite
//! device and a server.
//!
//! v0.1 scope: protocol types, a codec that reads/writes frames over
//! any `AsyncRead`/`AsyncWrite`, and a TCP server that accepts
//! connections and forwards parsed events onto a channel for the
//! caller to handle. STT, intent dispatch, and TTS land in later PRs.
//!
//! ## Frame format
//!
//! ```text
//! {"type":"audio-chunk","data":{"rate":16000,"width":2,"channels":1},"payload_length":32}\n
//! <32 bytes of binary audio>
//! {"type":"audio-stop"}\n
//! ```

pub mod codec;
pub mod error;
pub mod event;
pub mod server;

pub use codec::{WyomingReader, WyomingWriter};
pub use error::{Error, Result};
pub use event::Event;
pub use server::WyomingServer;

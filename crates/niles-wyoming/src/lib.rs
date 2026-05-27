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
//! connections, forwards parsed events onto a channel, and supports
//! sending events (including framed PCM audio) back to connected
//! peers via [`WyomingSender`]. STT, intent dispatch, and TTS
//! synthesis land in later PRs.
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
pub mod session;

pub use codec::{WyomingReader, WyomingWriter};
pub use error::{Error, Result, SendError};
pub use event::Event;
pub use server::{WyomingSender, WyomingServer};
pub use session::{AudioFormat, AudioSession, SessionTracker};

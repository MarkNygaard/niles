//! Speech-to-text adapter layer.
//!
//! v0.1 ships one provider — Groq Whisper Large v3 Turbo — exposed
//! through [`WhisperClient::transcribe`]. The PCM-to-WAV helper lives
//! here because a later PR will accumulate audio bytes from Wyoming
//! `audio-chunk` events (between `audio-start` and `audio-stop`) and
//! feed them into this crate.
//!
//! No `Stt` trait yet — per repo convention, traits land alongside
//! their second implementation, not the first.

mod error;
mod wav;
mod whisper;

pub use error::{Error, Result};
pub use wav::{PcmFormat, pcm_to_wav};
pub use whisper::{Transcript, WhisperClient, WhisperConfig};

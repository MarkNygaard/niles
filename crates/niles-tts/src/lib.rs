//! Text-to-speech adapter layer.
//!
//! v0.1 ships one provider — Piper, self-hosted in-cluster — exposed
//! through [`PiperClient::synthesize`]. Text in, WAV bytes out.
//!
//! No `Tts` trait yet — per repo convention, traits land alongside
//! their second implementation, not the first.

mod error;
mod piper;

pub use error::{Error, Result};
pub use piper::{PiperClient, PiperConfig, Synthesis};

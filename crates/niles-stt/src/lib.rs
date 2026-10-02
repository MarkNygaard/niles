//! Speech-to-text adapter layer.
//!
//! Two providers: anything that speaks the OpenAI transcription API
//! (Groq's Whisper, by default) through [`WhisperClient`], and
//! ElevenLabs Scribe through [`ScribeClient`]. [`SttClient`] is the one
//! the rest of Niles holds.

mod error;
mod scribe;
mod wav;
mod whisper;

pub use error::{Error, Result};
pub use scribe::{ScribeClient, ScribeConfig};
pub use wav::{PcmFormat, pcm_to_wav};
pub use whisper::{Confidence, Transcript, WhisperClient, WhisperConfig};

/// Whichever speech-to-text the config points at.
///
/// An enum rather than a trait object: there are two, both are known
/// here, and an async method on a `dyn` trait costs a boxed future per
/// utterance for no flexibility anybody needs.
pub enum SttClient {
    Whisper(WhisperClient),
    Scribe(ScribeClient),
}

impl SttClient {
    /// Which one this is, for the log line that says what was heard.
    pub fn name(&self) -> &'static str {
        match self {
            SttClient::Whisper(_) => "whisper",
            SttClient::Scribe(_) => "scribe",
        }
    }

    pub async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<Transcript> {
        match self {
            SttClient::Whisper(c) => c.transcribe(audio, filename).await,
            SttClient::Scribe(c) => c.transcribe(audio, filename).await,
        }
    }
}

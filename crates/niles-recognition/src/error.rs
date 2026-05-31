//! Error types for niles-recognition.

use std::path::PathBuf;
use thiserror::Error;

/// Errors surfaced by niles-recognition's public API.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("loading ONNX model {}: {source}", path.display())]
    LoadModel {
        path: PathBuf,
        #[source]
        source: ort::Error,
    },

    #[error("model {} has unexpected output shape {actual:?}; expected [_, 192]", path.display())]
    UnexpectedOutputShape { path: PathBuf, actual: Vec<i64> },

    #[error("model {} has unsupported input shape {actual:?}; expected [_, samples] or [_, 80, frames]", path.display())]
    UnsupportedInputShape { path: PathBuf, actual: Vec<i64> },

    #[error("model {} has unsupported input type {actual}; expected f32", path.display())]
    UnsupportedInputType { path: PathBuf, actual: String },

    #[error("model {} has unexpected output type {actual}; expected f32", path.display())]
    UnexpectedOutputType { path: PathBuf, actual: String },

    #[error("expected 16000 Hz audio, got {got}")]
    WrongSampleRate { got: u32 },

    #[error("audio clip too short ({samples} samples = {seconds:.2}s, need >= 0.5s)")]
    AudioTooShort { samples: usize, seconds: f32 },

    #[error("audio clip too long ({seconds:.2}s, max 10s)")]
    AudioTooLong { seconds: f32 },

    #[error("inference failed: {source}")]
    Inference {
        #[source]
        source: ort::Error,
    },

    #[error("inference returned unexpected embedding length {actual}; expected 192")]
    UnexpectedEmbeddingLength { actual: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

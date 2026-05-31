//! ECAPA-TDNN ONNX-Runtime embedder.

use std::path::PathBuf;
use std::sync::Mutex;

use ort::ep::ExecutionProvider;
use ort::session::Session;
use ort::value::{Tensor, ValueType};

use crate::error::{Error, Result};
use crate::preprocess;
use crate::similarity::l2_normalize;

pub struct EmbedderConfig {
    pub model_path: PathBuf,
    pub use_gpu: bool,
}

enum InputKind {
    /// Model expects raw PCM as f32 in shape `[1, samples]`.
    RawPcm,
    /// Model expects log-mel features in shape `[1, 80, frames]`.
    LogMel,
}

pub struct EcapaTdnnEmbedder {
    session: Mutex<Session>,
    input_name: String,
    output_name: String,
    input_kind: InputKind,
    #[allow(dead_code)]
    model_path: PathBuf,
}

impl EcapaTdnnEmbedder {
    pub fn new(cfg: &EmbedderConfig) -> Result<Self> {
        let load_err = |source| Error::LoadModel {
            path: cfg.model_path.clone(),
            source,
        };

        let mut builder = Session::builder().map_err(load_err)?;

        if cfg.use_gpu {
            let cuda = ort::ep::CUDA::default();
            if cuda.is_available().unwrap_or(false) {
                builder = builder
                    .with_execution_providers([cuda.build()])
                    .map_err(|source| load_err(source.into()))?;
            } else {
                tracing::info!(target: "niles_recognition", "GPU execution provider unavailable, falling back to CPU");
            }
        }

        let session = builder
            .commit_from_file(&cfg.model_path)
            .map_err(load_err)?;

        // Validate inputs/outputs
        let input = session
            .inputs()
            .first()
            .ok_or_else(|| Error::UnsupportedInputShape {
                path: cfg.model_path.clone(),
                actual: vec![],
            })?;
        let input_name = input.name().to_string();

        let input_kind = match input.dtype() {
            ValueType::Tensor { shape, .. } => {
                let dims: Vec<i64> = shape.iter().copied().collect();
                match dims.len() {
                    2 => InputKind::RawPcm,
                    3 if dims.get(1).copied().unwrap_or(-1) == 80 => InputKind::LogMel,
                    _ => {
                        return Err(Error::UnsupportedInputShape {
                            path: cfg.model_path.clone(),
                            actual: dims,
                        });
                    }
                }
            }
            _ => {
                return Err(Error::UnsupportedInputShape {
                    path: cfg.model_path.clone(),
                    actual: vec![],
                });
            }
        };

        let output = session
            .outputs()
            .first()
            .ok_or_else(|| Error::UnexpectedOutputShape {
                path: cfg.model_path.clone(),
                actual: vec![],
            })?;
        let output_name = output.name().to_string();

        match output.dtype() {
            ValueType::Tensor { shape, .. } => {
                let last_dim = shape.last().copied().unwrap_or(-1);
                if last_dim != 192 && last_dim != -1 {
                    let actual = shape.iter().copied().collect();
                    return Err(Error::UnexpectedOutputShape {
                        path: cfg.model_path.clone(),
                        actual,
                    });
                }
            }
            _ => {
                return Err(Error::UnexpectedOutputShape {
                    path: cfg.model_path.clone(),
                    actual: vec![],
                });
            }
        }

        Ok(Self {
            session: Mutex::new(session),
            input_name,
            output_name,
            input_kind,
            model_path: cfg.model_path.clone(),
        })
    }

    pub fn embedding_dim(&self) -> usize {
        192
    }

    pub fn extract(&self, pcm: &[i16], sample_rate_hz: u32) -> Result<Vec<f32>> {
        if sample_rate_hz != 16_000 {
            return Err(Error::WrongSampleRate {
                got: sample_rate_hz,
            });
        }

        let samples = pcm.len();
        let seconds = samples as f32 / 16_000.0;
        if samples < 8_000 {
            return Err(Error::AudioTooShort { samples, seconds });
        }
        if samples > 160_000 {
            return Err(Error::AudioTooLong { seconds });
        }

        let pcm_f32: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();

        let mut session = self.session.lock().unwrap();

        let input_tensor = match self.input_kind {
            InputKind::RawPcm => {
                let shape = vec![1i64, samples as i64];
                Tensor::from_array((shape, pcm_f32))
                    .map_err(|source| Error::Inference { source })?
            }
            InputKind::LogMel => {
                let log_mel = preprocess::log_mel(&pcm_f32);
                let n_frames = preprocess::num_frames(samples);
                let shape = vec![1i64, 80i64, n_frames as i64];
                Tensor::from_array((shape, log_mel))
                    .map_err(|source| Error::Inference { source })?
            }
        };

        let outputs = session
            .run(ort::inputs![self.input_name.as_str() => input_tensor])
            .map_err(|source| Error::Inference { source })?;

        let output = &outputs[self.output_name.as_str()];
        let view = output
            .try_extract_array::<f32>()
            .map_err(|source| Error::Inference { source })?;

        let mut embedding: Vec<f32> = view.iter().copied().collect();
        l2_normalize(&mut embedding);
        Ok(embedding)
    }
}

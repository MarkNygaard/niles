//! Gated integration tests for niles-recognition.
//!
//! Set `NILES_ECAPA_MODEL_PATH=/abs/path/to/model.onnx` to run.
//! Without it, every test prints "(skipping)" and returns early.

use std::path::PathBuf;

use niles_recognition::{EcapaTdnnEmbedder, EmbedderConfig, cosine_similarity};

fn model_path() -> Option<PathBuf> {
    std::env::var_os("NILES_ECAPA_MODEL_PATH").map(PathBuf::from)
}

fn test_cfg(path: PathBuf) -> EmbedderConfig {
    EmbedderConfig {
        model_path: path,
        use_gpu: false,
    }
}

macro_rules! require_model {
    () => {
        match model_path() {
            Some(p) => p,
            None => {
                println!("(NILES_ECAPA_MODEL_PATH not set; skipping)");
                return;
            }
        }
    };
}

/// Generate a 2-second 440 Hz sine wave at 16 kHz, i16 range.
fn sine_wave_2s() -> Vec<i16> {
    let sample_rate = 16_000;
    let duration = 2.0;
    let samples = (sample_rate as f32 * duration) as usize;
    let freq = 440.0;
    (0..samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let sample = (t * freq * 2.0 * std::f32::consts::PI).sin() * (i16::MAX as f32 * 0.5);
            sample as i16
        })
        .collect()
}

#[test]
fn extract_returns_192_dim() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = sine_wave_2s();
    let embedding = embedder.extract(&pcm, 16_000).expect("extract failed");
    assert_eq!(embedding.len(), 192);
}

#[test]
fn embedding_is_unit_norm() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = sine_wave_2s();
    let embedding = embedder.extract(&pcm, 16_000).expect("extract failed");

    let sim = cosine_similarity(&embedding, &embedding);
    assert!(
        (sim - 1.0).abs() < 1e-5,
        "cosine similarity with self should be ~1.0, got {sim}"
    );

    let l2_norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (l2_norm - 1.0).abs() < 1e-5,
        "L2 norm should be ~1.0, got {l2_norm}"
    );
}

#[test]
fn wrong_sample_rate_errors() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = sine_wave_2s();
    let err = embedder.extract(&pcm, 8_000).unwrap_err();
    assert!(matches!(
        err,
        niles_recognition::Error::WrongSampleRate { got: 8000 }
    ));
}

#[test]
fn too_short_errors() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = vec![0i16; 1_000];
    let err = embedder.extract(&pcm, 16_000).unwrap_err();
    assert!(matches!(
        err,
        niles_recognition::Error::AudioTooShort { .. }
    ));
}

#[test]
fn too_long_errors() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = vec![0i16; 200_000];
    let err = embedder.extract(&pcm, 16_000).unwrap_err();
    assert!(matches!(err, niles_recognition::Error::AudioTooLong { .. }));
}

#[test]
fn boundary_8000_samples_ok() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = vec![0i16; 8_000];
    let _ = embedder
        .extract(&pcm, 16_000)
        .expect("exactly 0.5s should be allowed");
}

#[test]
fn boundary_160000_samples_ok() {
    let path = require_model!();
    let embedder = EcapaTdnnEmbedder::new(&test_cfg(path)).expect("failed to load model");
    let pcm = vec![0i16; 160_000];
    let _ = embedder
        .extract(&pcm, 16_000)
        .expect("exactly 10s should be allowed");
}

// TODO(follow-up PR): Add real-speaker separation tests once WAV fixtures are
// available. Two clips from the same speaker should have cosine similarity > 0.5,
// while clips from different speakers should have cosine similarity < 0.5.
// ECAPA-TDNN's separation varies per dataset, so tight bounds are not asserted.

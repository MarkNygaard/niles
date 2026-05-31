//! Speaker recognition (ECAPA-TDNN) configuration.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

pub use niles_recognition::MatchStrategy;

/// `[recognition]` section. When `model_path` is `None`, the
/// recognition subsystem is disabled — niles starts without
/// instantiating the embedder.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecognitionConfig {
    pub model_path: Option<PathBuf>,
    #[serde(default)]
    pub use_gpu: bool,
    #[serde(default)]
    pub matcher: MatcherConfig,
}

impl RecognitionConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(p) = &self.model_path {
            if p.as_os_str().is_empty() {
                return Err(Error::InvalidSection {
                    section: "recognition",
                    reason: "model_path must not be empty if present".into(),
                });
            }
            // Use `has_root` instead of `is_absolute` so the same
            // config TOML works cross-platform — `/var/niles/ecapa.onnx`
            // is rooted (and thus accepted) on both Linux and Windows,
            // even though Windows `is_absolute` requires a drive prefix.
            if !p.has_root() {
                return Err(Error::InvalidSection {
                    section: "recognition",
                    reason: "model_path must be absolute".into(),
                });
            }
        }

        if !(0.0..=1.0).contains(&self.matcher.threshold) {
            return Err(Error::InvalidSection {
                section: "recognition.matcher",
                reason: "threshold must be in [0.0, 1.0]".into(),
            });
        }
        if let Some(p) = &self.matcher.enrollment_dir {
            if p.as_os_str().is_empty() {
                return Err(Error::InvalidSection {
                    section: "recognition.matcher",
                    reason: "enrollment_dir must not be empty if present".into(),
                });
            }
            if !p.has_root() {
                return Err(Error::InvalidSection {
                    section: "recognition.matcher",
                    reason: "enrollment_dir must be absolute".into(),
                });
            }
        }

        Ok(())
    }
}

/// `[recognition.matcher]` subsection.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatcherConfig {
    pub enrollment_dir: Option<PathBuf>,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default)]
    pub strategy: MatchStrategy,
}

impl Default for MatcherConfig {
    fn default() -> Self {
        Self {
            enrollment_dir: None,
            threshold: default_threshold(),
            strategy: MatchStrategy::default(),
        }
    }
}

fn default_threshold() -> f32 {
    0.65
}

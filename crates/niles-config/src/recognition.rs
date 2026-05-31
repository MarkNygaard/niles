//! Speaker recognition (ECAPA-TDNN) configuration.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// `[recognition]` section. When `model_path` is `None`, the
/// recognition subsystem is disabled — niles starts without
/// instantiating the embedder.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecognitionConfig {
    pub model_path: Option<PathBuf>,
    #[serde(default)]
    pub use_gpu: bool,
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
            if !p.is_absolute() {
                return Err(Error::InvalidSection {
                    section: "recognition",
                    reason: "model_path must be absolute".into(),
                });
            }
        }
        Ok(())
    }
}

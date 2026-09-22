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
    #[serde(default)]
    pub enabled: bool,
    pub model_path: Option<PathBuf>,
    #[serde(default)]
    pub use_gpu: bool,
    #[serde(default)]
    pub matcher: MatcherConfig,
    /// Refuse anything said by a voice Niles does not know.
    ///
    /// The lock, and it is one switch rather than two on purpose: the
    /// question a household actually asks is "does Niles trust this
    /// voice", and the answer decides both whether it acts and whether
    /// it will learn a new name.
    ///
    /// Off by default. On, a stranger is told they are not recognised
    /// and nothing happens — which includes "I am Sofia", because an
    /// enrolment nobody in the house asked for is how a television
    /// becomes a resident. Adding somebody means switching this off
    /// for as long as it takes them to say their name.
    #[serde(default)]
    pub known_voices_only: bool,
}

impl RecognitionConfig {
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.model_path.is_none() {
            return Err(Error::InvalidSection {
                section: "recognition",
                reason: "model_path is required when enabled".into(),
            });
        }
        // No enrollment_dir check: enrolled voices go to the database
        // when there is one, and a voice print is under a kilobyte —
        // far too little to justify a volume. The binary reports it if
        // neither is configured, where it can see both sections.

        if let Some(p) = &self.model_path {
            if p.as_os_str().is_empty() {
                return Err(Error::InvalidSection {
                    section: "recognition",
                    reason: "model_path must not be empty if present".into(),
                });
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn disabled_with_no_paths_ok() {
        let cfg = RecognitionConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn enabled_without_model_path_err() {
        let cfg = RecognitionConfig {
            enabled: true,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("model_path is required when enabled"), "{err}");
    }

    #[test]
    fn enabled_without_an_enrollment_dir_is_fine() {
        // Enrolled voices go to the database when there is one, and
        // this section cannot see whether there is. The binary decides,
        // where it can read both.
        let cfg = RecognitionConfig {
            enabled: true,
            model_path: Some(PathBuf::from("/models/ecapa.onnx")),
            ..Default::default()
        };
        cfg.validate().expect("a database may be holding them");
    }

    #[test]
    fn enabled_with_absolute_paths_ok() {
        let cfg = RecognitionConfig {
            enabled: true,
            model_path: Some(PathBuf::from("/models/ecapa.onnx")),
            use_gpu: false,
            matcher: MatcherConfig {
                enrollment_dir: Some(PathBuf::from("/data/enrollments")),
                threshold: 0.65,
                strategy: MatchStrategy::default(),
            },
            known_voices_only: false,
        };
        assert!(cfg.validate().is_ok());
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn the_door_is_open_until_somebody_closes_it() {
        // A house that has never heard of this setting must not have
        // been locked by upgrading.
        assert!(!RecognitionConfig::default().known_voices_only);
    }

    #[test]
    fn it_is_read_off_the_config() {
        let cfg: RecognitionConfig = toml::from_str("known_voices_only = true").expect("valid");
        assert!(cfg.known_voices_only);
    }

    #[test]
    fn locking_does_not_require_recognition_to_be_configured_here() {
        // Validation stays about paths. Whether the lock can do
        // anything is a question for the running system — which
        // answers it by asking whether anybody is enrolled — not for
        // a file that cannot see the roster.
        let cfg: RecognitionConfig = toml::from_str("known_voices_only = true").expect("valid");
        assert!(cfg.validate().is_ok());
    }
}

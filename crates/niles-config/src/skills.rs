//! Skills configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_skill_max_chars() -> usize {
    100_000
}

fn default_curator_enabled() -> bool {
    true
}

fn default_interval_hours() -> u64 {
    24
}

fn default_stale_after_days() -> u64 {
    30
}

fn default_archive_after_days() -> u64 {
    90
}

fn default_review_enabled() -> bool {
    true
}

fn default_review_max_iters() -> u8 {
    4
}

/// `[skills.review]` subsection.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsReviewConfig {
    #[serde(default = "default_review_enabled")]
    pub enabled: bool,
    #[serde(default = "default_review_max_iters")]
    pub max_iters: u8,
}

impl Default for SkillsReviewConfig {
    fn default() -> Self {
        Self {
            enabled: default_review_enabled(),
            max_iters: default_review_max_iters(),
        }
    }
}

/// `[skills.curator]` subsection.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsCuratorConfig {
    #[serde(default = "default_curator_enabled")]
    pub enabled: bool,
    #[serde(default = "default_interval_hours")]
    pub interval_hours: u64,
    #[serde(default = "default_stale_after_days")]
    pub stale_after_days: u64,
    #[serde(default = "default_archive_after_days")]
    pub archive_after_days: u64,
}

impl Default for SkillsCuratorConfig {
    fn default() -> Self {
        Self {
            enabled: default_curator_enabled(),
            interval_hours: default_interval_hours(),
            stale_after_days: default_stale_after_days(),
            archive_after_days: default_archive_after_days(),
        }
    }
}

/// `[skills]` section of the config file.
///
/// Optional, and only tuning. Skills are always on, and kept in the
/// database when there is one; they used to need a `directory` here,
/// and without it Niles could not learn anything.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsConfig {
    /// Maximum number of characters in a skill body.
    #[serde(default = "default_skill_max_chars")]
    pub skill_max_chars: usize,
    #[serde(default)]
    pub curator: SkillsCuratorConfig,
    #[serde(default)]
    pub review: SkillsReviewConfig,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skill_max_chars: default_skill_max_chars(),
            curator: SkillsCuratorConfig::default(),
            review: SkillsReviewConfig::default(),
        }
    }
}

impl SkillsConfig {
    pub fn validate(&self) -> Result<()> {
        if self.skill_max_chars == 0 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "skill_max_chars must be > 0".into(),
            });
        }
        if self.curator.interval_hours == 0 || self.curator.interval_hours > 24 * 365 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "curator.interval_hours must be in 1..=8760".into(),
            });
        }
        if self.curator.stale_after_days == 0 || self.curator.stale_after_days > 3650 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "curator.stale_after_days must be in 1..=3650".into(),
            });
        }
        if self.curator.archive_after_days == 0 || self.curator.archive_after_days > 3650 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "curator.archive_after_days must be in 1..=3650".into(),
            });
        }
        if self.curator.archive_after_days < self.curator.stale_after_days {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "curator.archive_after_days must be >= stale_after_days".into(),
            });
        }
        if self.review.max_iters == 0 || self.review.max_iters > 20 {
            return Err(Error::InvalidSection {
                section: "skills",
                reason: "review.max_iters must be in 1..=20".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_defaults() {
        let cfg = SkillsConfig::default();
        assert!(cfg.review.enabled);
        assert_eq!(cfg.review.max_iters, 4);
    }

    #[test]
    fn review_toml_parsing() {
        let toml = r#"
            [review]
            enabled = false
            max_iters = 8
        "#;
        let cfg: SkillsConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.review.enabled);
        assert_eq!(cfg.review.max_iters, 8);
    }

    #[test]
    fn validate_rejects_max_iters_zero() {
        let cfg = SkillsConfig {
            review: SkillsReviewConfig {
                max_iters: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, Error::InvalidSection { section, reason } if section == "skills" && reason.contains("max_iters"))
        );
    }

    #[test]
    fn validate_rejects_max_iters_over_20() {
        let cfg = SkillsConfig {
            review: SkillsReviewConfig {
                max_iters: 21,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, Error::InvalidSection { section, reason } if section == "skills" && reason.contains("max_iters"))
        );
    }

    #[test]
    fn validate_accepts_max_iters_in_range() {
        for max_iters in [1u8, 10, 20] {
            let cfg = SkillsConfig {
                review: SkillsReviewConfig {
                    max_iters,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                cfg.validate().is_ok(),
                "max_iters={max_iters} should be valid"
            );
        }
    }
}

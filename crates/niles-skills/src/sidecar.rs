//! `.usage.json` sidecar telemetry for each skill.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::Result;
use crate::util;

/// Who created the skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    AgentCreated,
    UserCreated,
}

/// Telemetry sidecar stored as `<skill-dir>/.usage.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sidecar {
    pub created_at: DateTime<Utc>,
    pub provenance: Provenance,
    #[serde(default)]
    pub patch_count: u64,
    #[serde(default)]
    pub last_patched_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pinned: bool,
}

impl Sidecar {
    pub fn new(provenance: Provenance) -> Self {
        Self {
            created_at: Utc::now(),
            provenance,
            patch_count: 0,
            last_patched_at: None,
            usage_count: 0,
            last_used_at: None,
            pinned: false,
        }
    }

    /// Latest activity timestamp, ignoring `created_at`.
    /// Returns `None` when both activity fields are unset.
    pub fn latest_activity_at(&self) -> Option<DateTime<Utc>> {
        [self.last_used_at, self.last_patched_at]
            .into_iter()
            .flatten()
            .max()
    }

    /// Read the sidecar from `path`.
    pub fn read(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let sidecar: Self = serde_json::from_str(&raw)?;
        Ok(sidecar)
    }

    /// Write the sidecar to `path` atomically.
    pub fn write(&self, path: &Path) -> Result<()> {
        let raw = serde_json::to_vec_pretty(self)?;
        util::atomic_write(path, &raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    #[test]
    fn serde_round_trip() {
        let sidecar = Sidecar {
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
            provenance: Provenance::AgentCreated,
            patch_count: 3,
            last_patched_at: Some(Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap()),
            usage_count: 7,
            last_used_at: Some(Utc.with_ymd_and_hms(2026, 1, 3, 12, 0, 0).unwrap()),
            pinned: true,
        };

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".usage.json");
        sidecar.write(&path).unwrap();
        let read_back = Sidecar::read(&path).unwrap();
        assert_eq!(sidecar, read_back);
    }

    #[test]
    fn defaults_when_fields_missing() {
        let json = r#"{"created_at":"2026-01-01T12:00:00Z","provenance":"agent-created"}"#;
        let sidecar: Sidecar = serde_json::from_str(json).unwrap();
        assert_eq!(sidecar.patch_count, 0);
        assert_eq!(sidecar.usage_count, 0);
        assert!(!sidecar.pinned);
        assert!(sidecar.last_patched_at.is_none());
        assert!(sidecar.last_used_at.is_none());
    }

    #[test]
    fn latest_activity_all_none() {
        let sidecar = Sidecar::new(Provenance::UserCreated);
        assert!(sidecar.latest_activity_at().is_none());
    }

    #[test]
    fn latest_activity_max_of_set_fields() {
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();

        let mut sidecar = Sidecar::new(Provenance::UserCreated);
        sidecar.last_used_at = Some(t1);
        assert_eq!(sidecar.latest_activity_at(), Some(t1));

        sidecar.last_patched_at = Some(t2);
        assert_eq!(sidecar.latest_activity_at(), Some(t2));

        sidecar.last_used_at = Some(t2);
        sidecar.last_patched_at = Some(t1);
        assert_eq!(sidecar.latest_activity_at(), Some(t2));
    }

    #[test]
    fn latest_activity_ignores_created_at() {
        let created = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::UserCreated);
        sidecar.created_at = created;
        assert!(sidecar.latest_activity_at().is_none());
    }
}

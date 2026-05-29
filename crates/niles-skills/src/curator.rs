//! Skill lifecycle curator — pure stale/archive transitions over
//! the on-disk sidecar telemetry. No LLM involvement; that's PR 4.

use chrono::{DateTime, Duration, Utc};

use crate::error::Result;
use crate::sidecar::{Provenance, SkillStatus};
use crate::store::{SkillStatusFilter, SkillStore};

#[derive(Debug, Clone, Copy)]
pub struct CuratorThresholds {
    pub stale_after: Duration,
    pub archive_after: Duration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransitionReport {
    pub examined: usize,
    pub became_stale: usize,
    pub became_archived: usize,
    pub revived: usize,
    pub skipped_pinned: usize,
    pub skipped_user_created: usize,
}

/// Returns the status this skill *should* have based on age,
/// without considering provenance or pinning. Caller filters
/// those upstream.
fn target_status_for_age(age: Duration, thresholds: CuratorThresholds) -> SkillStatus {
    if age >= thresholds.archive_after {
        SkillStatus::Archived
    } else if age >= thresholds.stale_after {
        SkillStatus::Stale
    } else {
        SkillStatus::Active
    }
}

/// Walk every skill and apply automatic stale/archive transitions.
pub fn apply_automatic_transitions(
    store: &SkillStore,
    now: DateTime<Utc>,
    thresholds: CuratorThresholds,
) -> Result<TransitionReport> {
    let mut report = TransitionReport::default();
    let summaries = store.list_summaries_filtered(SkillStatusFilter::All)?;
    for s in summaries {
        report.examined += 1;
        if matches!(s.provenance, Provenance::UserCreated) {
            report.skipped_user_created += 1;
            continue;
        }
        if s.pinned {
            report.skipped_pinned += 1;
            continue;
        }
        let target = match store.update_status(&s.name, |sidecar| {
            // Re-check mutable sidecar fields under the store lock to avoid
            // transitioning a skill that was pinned (or provenance-changed
            // via manual edits) after list_summaries() returned.
            if matches!(sidecar.provenance, Provenance::UserCreated) || sidecar.pinned {
                return None;
            }
            let last_activity = sidecar.latest_activity_at().unwrap_or(sidecar.created_at);
            let age = now.signed_duration_since(last_activity);
            let target = target_status_for_age(age, thresholds);
            if target == sidecar.status {
                None
            } else {
                Some(target)
            }
        }) {
            Ok(Some(t)) => t,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(skill = %s.name, error = %e,
                    "curator: failed to update status, skipping");
                continue;
            }
        };
        match target {
            SkillStatus::Stale => report.became_stale += 1,
            SkillStatus::Archived => report.became_archived += 1,
            SkillStatus::Active => report.revived += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::Sidecar;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn store(tmp: &TempDir) -> SkillStore {
        SkillStore::open(tmp.path(), 100_000, 1_048_576).unwrap()
    }

    // ------------------------------------------------------------------
    // Pure transition-rule tests
    // ------------------------------------------------------------------

    #[test]
    fn active_within_threshold_stays_active() {
        let t = CuratorThresholds {
            stale_after: Duration::days(30),
            archive_after: Duration::days(90),
        };
        assert_eq!(
            target_status_for_age(Duration::days(5), t),
            SkillStatus::Active
        );
    }

    #[test]
    fn between_thresholds_becomes_stale() {
        let t = CuratorThresholds {
            stale_after: Duration::days(30),
            archive_after: Duration::days(90),
        };
        assert_eq!(
            target_status_for_age(Duration::days(60), t),
            SkillStatus::Stale
        );
    }

    #[test]
    fn past_archive_threshold_becomes_archived() {
        let t = CuratorThresholds {
            stale_after: Duration::days(30),
            archive_after: Duration::days(90),
        };
        assert_eq!(
            target_status_for_age(Duration::days(100), t),
            SkillStatus::Archived
        );
    }

    // ------------------------------------------------------------------
    // Store-walking tests
    // ------------------------------------------------------------------

    #[test]
    fn agent_skill_60_days_idle_becomes_stale() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.last_used_at = Some(created);
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap(); // ~73 days later
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.became_stale, 1);
        assert_eq!(report.became_archived, 0);
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Stale);
    }

    #[test]
    fn agent_skill_100_days_idle_becomes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.last_used_at = Some(created);
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap(); // ~104 days later
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.became_stale, 0);
        assert_eq!(report.became_archived, 1);
        assert_eq!(
            store.load("s").unwrap().sidecar.status,
            SkillStatus::Archived
        );
    }

    #[test]
    fn pinned_agent_skill_100_days_idle_unchanged() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.last_used_at = Some(created);
        sidecar.pinned = true;
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.skipped_pinned, 1);
        assert_eq!(report.became_archived, 0);
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn user_created_skill_100_days_idle_unchanged() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::UserCreated);
        sidecar.created_at = created;
        sidecar.last_used_at = Some(created);
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.skipped_user_created, 1);
        assert_eq!(report.became_archived, 0);
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn never_used_agent_skill_with_old_created_at_becomes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        // No activity fields set — latest_activity_at() returns None.
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.became_archived, 1);
        assert_eq!(
            store.load("s").unwrap().sidecar.status,
            SkillStatus::Archived
        );
    }

    #[test]
    fn stale_skill_with_fresh_activity_revives_to_active() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.status = SkillStatus::Stale;
        sidecar.last_used_at = Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()); // 5 days ago
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.revived, 1);
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn report_counts_match_transitions() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);

        // 1. agent, idle 60 days → Stale
        store
            .create("agent-stale", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let mut sc = Sidecar::new(Provenance::AgentCreated);
        sc.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        sc.last_used_at = Some(Utc.with_ymd_and_hms(2026, 2, 14, 12, 0, 0).unwrap());
        sc.write(&tmp.path().join("agent-stale").join(".usage.json"))
            .unwrap();

        // 2. agent, idle 100 days → Archived
        store
            .create(
                "agent-archived",
                "D",
                "0.1.0",
                "B",
                Provenance::AgentCreated,
            )
            .unwrap();
        let mut sc = Sidecar::new(Provenance::AgentCreated);
        sc.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        sc.last_used_at = Some(sc.created_at);
        sc.write(&tmp.path().join("agent-archived").join(".usage.json"))
            .unwrap();

        // 3. agent, pinned, idle 100 days → skipped
        store
            .create("agent-pinned", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let mut sc = Sidecar::new(Provenance::AgentCreated);
        sc.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        sc.last_used_at = Some(sc.created_at);
        sc.pinned = true;
        sc.write(&tmp.path().join("agent-pinned").join(".usage.json"))
            .unwrap();

        // 4. user-created, idle 100 days → skipped
        store
            .create("user-idle", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        let mut sc = Sidecar::new(Provenance::UserCreated);
        sc.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        sc.last_used_at = Some(sc.created_at);
        sc.write(&tmp.path().join("user-idle").join(".usage.json"))
            .unwrap();

        // 5. agent, stale on disk, fresh activity → revived
        store
            .create("agent-revived", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let mut sc = Sidecar::new(Provenance::AgentCreated);
        sc.created_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        sc.status = SkillStatus::Stale;
        sc.last_used_at = Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap());
        sc.write(&tmp.path().join("agent-revived").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();

        assert_eq!(report.examined, 5);
        assert_eq!(report.became_stale, 1);
        assert_eq!(report.became_archived, 1);
        assert_eq!(report.revived, 1);
        assert_eq!(report.skipped_pinned, 1);
        assert_eq!(report.skipped_user_created, 1);
    }

    #[test]
    fn equal_thresholds_skips_stale_goes_straight_to_archived() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap(); // ~104 days
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(90),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.became_stale, 0);
        assert_eq!(report.became_archived, 1);
        assert_eq!(
            store.load("s").unwrap().sidecar.status,
            SkillStatus::Archived
        );
    }

    #[test]
    fn archived_skill_with_fresh_activity_revives_to_active() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.status = SkillStatus::Archived;
        sidecar.last_used_at = Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()); // 5 days ago
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 15, 12, 0, 0).unwrap();
        let report = apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(report.revived, 1);
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn bump_after_curator_set_to_stale_resets_to_active() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let created = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut sidecar = Sidecar::new(Provenance::AgentCreated);
        sidecar.created_at = created;
        sidecar.last_used_at = Some(created);
        sidecar
            .write(&tmp.path().join("s").join(".usage.json"))
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap();
        apply_automatic_transitions(
            &store,
            now,
            CuratorThresholds {
                stale_after: Duration::days(30),
                archive_after: Duration::days(90),
            },
        )
        .unwrap();
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Stale);

        store.bump_use("s").unwrap();
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }
}

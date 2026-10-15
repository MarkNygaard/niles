//! Where each person's training is kept: Postgres in the house
//! (niles-db), in memory for tests.
//!
//! Saving is an upsert by key — a workout by (person, source, external
//! id), a day by (person, date) — that touches nothing when nothing
//! changed. Health Auto Export sends the same days over and over; the
//! second time must leave everything as the first left it.

use crate::error::Result;
use crate::model::{Activity, Kind, Wellness};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use std::sync::Mutex;

/// What a save did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Saved {
    pub added: usize,
    pub updated: usize,
}

/// What the profile card shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Overview {
    /// When the current key was made; `None` with no key.
    pub key_made_at: Option<DateTime<Utc>>,
    pub last_received_at: Option<DateTime<Utc>>,
    pub runs: u64,
    pub activities: u64,
    pub wellness_days: u64,
}

#[async_trait]
pub trait TrainingStore: Send + Sync {
    async fn save_activities(&self, person: &str, activities: &[Activity]) -> Result<Saved>;
    /// Each day merged into the one kept: a field the day does not
    /// carry keeps what was there.
    async fn save_wellness(&self, person: &str, days: &[Wellness]) -> Result<Saved>;
    /// Oldest first, `from` inclusive and `to` exclusive.
    async fn activities(
        &self,
        person: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        kind: Option<Kind>,
    ) -> Result<Vec<Activity>>;
    /// Oldest first, both ends inclusive.
    async fn wellness(&self, person: &str, from: NaiveDate, to: NaiveDate)
    -> Result<Vec<Wellness>>;
    async fn overview(&self, person: &str) -> Result<Overview>;
    /// Make `hash` the person's one key, replacing any they had.
    async fn set_key(&self, person: &str, hash: &str) -> Result<()>;
    /// False when they had no key.
    async fn revoke_key(&self, person: &str) -> Result<bool>;
    /// Whose key hashes to `hash`.
    async fn person_for_key(&self, hash: &str) -> Result<Option<String>>;
    async fn received(&self, person: &str, at: DateTime<Utc>) -> Result<()>;
}

#[derive(Default)]
struct Kept {
    activities: Vec<(String, Activity)>,
    wellness: Vec<(String, Wellness)>,
    /// person, hash, made at, last received.
    keys: Vec<(String, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>)>,
}

#[derive(Default)]
pub struct MemoryTraining {
    kept: Mutex<Kept>,
}

impl MemoryTraining {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl TrainingStore for MemoryTraining {
    async fn save_activities(&self, person: &str, activities: &[Activity]) -> Result<Saved> {
        let mut kept = self.lock();
        let mut saved = Saved::default();
        for activity in activities {
            match kept.activities.iter_mut().find(|(p, a)| {
                p == person
                    && a.source == activity.source
                    && a.external_id == activity.external_id
            }) {
                Some((_, existing)) if existing == activity => {}
                Some((_, existing)) => {
                    *existing = activity.clone();
                    saved.updated += 1;
                }
                None => {
                    kept.activities.push((person.into(), activity.clone()));
                    saved.added += 1;
                }
            }
        }
        Ok(saved)
    }

    async fn save_wellness(&self, person: &str, days: &[Wellness]) -> Result<Saved> {
        let mut kept = self.lock();
        let mut saved = Saved::default();
        for day in days.iter().filter(|d| !d.is_empty()) {
            match kept
                .wellness
                .iter_mut()
                .find(|(p, w)| p == person && w.date == day.date)
            {
                Some((_, existing)) => {
                    let before = existing.clone();
                    existing.merge(day);
                    if *existing != before {
                        saved.updated += 1;
                    }
                }
                None => {
                    kept.wellness.push((person.into(), day.clone()));
                    saved.added += 1;
                }
            }
        }
        Ok(saved)
    }

    async fn activities(
        &self,
        person: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        kind: Option<Kind>,
    ) -> Result<Vec<Activity>> {
        let mut out: Vec<Activity> = self
            .lock()
            .activities
            .iter()
            .filter(|(p, a)| {
                p == person
                    && from.is_none_or(|f| a.started_at >= f)
                    && to.is_none_or(|t| a.started_at < t)
                    && kind.is_none_or(|k| a.kind == k)
            })
            .map(|(_, a)| a.clone())
            .collect();
        out.sort_by_key(|a| a.started_at);
        Ok(out)
    }

    async fn wellness(
        &self,
        person: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Wellness>> {
        let mut out: Vec<Wellness> = self
            .lock()
            .wellness
            .iter()
            .filter(|(p, w)| p == person && w.date >= from && w.date <= to)
            .map(|(_, w)| w.clone())
            .collect();
        out.sort_by_key(|w| w.date);
        Ok(out)
    }

    async fn overview(&self, person: &str) -> Result<Overview> {
        let kept = self.lock();
        let key = kept.keys.iter().find(|k| k.0 == person);
        let theirs = kept.activities.iter().filter(|(p, _)| p == person);
        Ok(Overview {
            key_made_at: key.and_then(|k| k.1.as_ref().and(k.2)),
            last_received_at: key.and_then(|k| k.3),
            runs: theirs.clone().filter(|(_, a)| a.kind == Kind::Run).count() as u64,
            activities: theirs.count() as u64,
            wellness_days: kept.wellness.iter().filter(|(p, _)| p == person).count() as u64,
        })
    }

    async fn set_key(&self, person: &str, hash: &str) -> Result<()> {
        let mut kept = self.lock();
        let now = Some(Utc::now());
        match kept.keys.iter_mut().find(|k| k.0 == person) {
            Some(k) => {
                k.1 = Some(hash.into());
                k.2 = now;
            }
            None => kept.keys.push((person.into(), Some(hash.into()), now, None)),
        }
        Ok(())
    }

    async fn revoke_key(&self, person: &str) -> Result<bool> {
        let mut kept = self.lock();
        Ok(match kept.keys.iter_mut().find(|k| k.0 == person) {
            Some(k) if k.1.is_some() => {
                k.1 = None;
                k.2 = None;
                true
            }
            _ => false,
        })
    }

    async fn person_for_key(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .keys
            .iter()
            .find(|k| k.1.as_deref().is_some_and(|h| crate::same_hash(h, hash)))
            .map(|k| k.0.clone()))
    }

    async fn received(&self, person: &str, at: DateTime<Utc>) -> Result<()> {
        let mut kept = self.lock();
        match kept.keys.iter_mut().find(|k| k.0 == person) {
            Some(k) => k.3 = Some(at),
            None => kept.keys.push((person.into(), None, None, Some(at))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> crate::hae::Batch {
        let body = serde_json::from_str(include_str!("../fixtures/hae-v2-metric.json")).unwrap();
        crate::hae::parse(&body, &chrono_tz::Europe::Copenhagen).unwrap()
    }

    #[tokio::test]
    async fn sending_the_same_data_twice_changes_nothing() {
        let store = MemoryTraining::new();
        let b = batch();
        let first = store.save_activities("person:majse", &b.activities).await.unwrap();
        let days = store.save_wellness("person:majse", &b.wellness).await.unwrap();
        assert_eq!(first, Saved { added: 1, updated: 0 });
        assert_eq!(days, Saved { added: 2, updated: 0 });

        let again = store.save_activities("person:majse", &b.activities).await.unwrap();
        let days = store.save_wellness("person:majse", &b.wellness).await.unwrap();
        assert_eq!(again, Saved::default());
        assert_eq!(days, Saved::default());
        let overview = store.overview("person:majse").await.unwrap();
        assert_eq!((overview.runs, overview.wellness_days), (1, 2));
    }

    #[tokio::test]
    async fn a_corrected_workout_is_updated_not_added() {
        let store = MemoryTraining::new();
        let mut activities = batch().activities;
        store.save_activities("p", &activities).await.unwrap();
        activities[0].distance_m = Some(5100.0);
        let saved = store.save_activities("p", &activities).await.unwrap();
        assert_eq!(saved, Saved { added: 0, updated: 1 });
    }

    #[tokio::test]
    async fn one_person_s_data_is_not_another_s() {
        let store = MemoryTraining::new();
        let b = batch();
        store.save_activities("person:majse", &b.activities).await.unwrap();
        store.save_wellness("person:majse", &b.wellness).await.unwrap();
        let theirs = store.activities("person:mark", None, None, None).await.unwrap();
        assert!(theirs.is_empty());
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        assert!(store.wellness("person:mark", day, day).await.unwrap().is_empty());
        // The same workout id for somebody else is their own row.
        let saved = store.save_activities("person:mark", &b.activities).await.unwrap();
        assert_eq!(saved.added, 1);
    }

    #[tokio::test]
    async fn a_key_is_found_by_its_hash_until_revoked() {
        let store = MemoryTraining::new();
        let hash = crate::hash_key("the key");
        store.set_key("person:majse", &hash).await.unwrap();
        assert_eq!(
            store.person_for_key(&hash).await.unwrap().as_deref(),
            Some("person:majse")
        );
        assert!(store.revoke_key("person:majse").await.unwrap());
        assert_eq!(store.person_for_key(&hash).await.unwrap(), None);
        assert!(!store.revoke_key("person:majse").await.unwrap());
    }
}

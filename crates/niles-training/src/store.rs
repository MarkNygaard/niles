//! Where each person's training is kept: Postgres in the house
//! (niles-db), in memory for tests.
//!
//! Saving a workout is an upsert by (person, source, external id) that
//! touches nothing when nothing changed, so fetching the same workout
//! again leaves everything as the first fetch left it.
//!
//! The Strava rows hold a person's app secret and tokens. Postgres
//! seals them; nothing that reads a [`StravaLink`] sends it anywhere
//! but to Strava.

use crate::error::Result;
use crate::model::{Activity, Athlete, Kind, StravaApp, StravaLink, StravaTokens};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Mutex;

/// What a save did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Saved {
    pub added: usize,
    pub updated: usize,
}

/// How much a person has kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Overview {
    pub runs: u64,
    pub activities: u64,
}

#[async_trait]
pub trait TrainingStore: Send + Sync {
    async fn save_activities(&self, person: &str, activities: &[Activity]) -> Result<Saved>;
    /// Oldest first, `from` inclusive and `to` exclusive.
    async fn activities(
        &self,
        person: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        kind: Option<Kind>,
    ) -> Result<Vec<Activity>>;
    async fn overview(&self, person: &str) -> Result<Overview>;
    /// When the newest workout from `source` started.
    async fn latest(&self, person: &str, source: &str) -> Result<Option<DateTime<Utc>>>;

    async fn strava(&self, person: &str) -> Result<Option<StravaLink>>;
    /// Everyone with tokens: whom the poll syncs.
    async fn strava_connected(&self) -> Result<Vec<String>>;
    /// The person's app, made or replaced. Their tokens are left alone.
    async fn set_strava_app(&self, person: &str, app: &StravaApp) -> Result<()>;
    /// The app, the tokens and the sync state. False when there was
    /// none. The workouts stay: they are the person's, not Strava's.
    async fn remove_strava(&self, person: &str) -> Result<bool>;
    /// `athlete` is `None` on a refresh, which leaves it as it was.
    async fn set_strava_tokens(
        &self,
        person: &str,
        tokens: &StravaTokens,
        athlete: Option<&Athlete>,
    ) -> Result<()>;
    /// The tokens and the athlete; the app stays.
    async fn clear_strava_tokens(&self, person: &str) -> Result<()>;
    /// A sync's outcome: `at` when it got through (otherwise the last
    /// one is kept), and why not when it did not.
    async fn strava_synced(
        &self,
        person: &str,
        at: Option<DateTime<Utc>>,
        error: Option<&str>,
    ) -> Result<()>;
}

#[derive(Default)]
struct Kept {
    activities: Vec<(String, Activity)>,
    strava: Vec<(String, StravaLink)>,
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

    fn with_link(&self, person: &str, change: impl FnOnce(&mut StravaLink)) {
        if let Some((_, link)) = self.lock().strava.iter_mut().find(|(p, _)| p == person) {
            change(link);
        }
    }
}

#[async_trait]
impl TrainingStore for MemoryTraining {
    async fn save_activities(&self, person: &str, activities: &[Activity]) -> Result<Saved> {
        let mut kept = self.lock();
        let mut saved = Saved::default();
        for activity in activities {
            match kept.activities.iter_mut().find(|(p, a)| {
                p == person && a.source == activity.source && a.external_id == activity.external_id
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

    async fn overview(&self, person: &str) -> Result<Overview> {
        let kept = self.lock();
        let theirs = kept.activities.iter().filter(|(p, _)| p == person);
        Ok(Overview {
            runs: theirs.clone().filter(|(_, a)| a.kind == Kind::Run).count() as u64,
            activities: theirs.count() as u64,
        })
    }

    async fn latest(&self, person: &str, source: &str) -> Result<Option<DateTime<Utc>>> {
        Ok(self
            .lock()
            .activities
            .iter()
            .filter(|(p, a)| p == person && a.source == source)
            .map(|(_, a)| a.started_at)
            .max())
    }

    async fn strava(&self, person: &str) -> Result<Option<StravaLink>> {
        Ok(self
            .lock()
            .strava
            .iter()
            .find(|(p, _)| p == person)
            .map(|(_, l)| l.clone()))
    }

    async fn strava_connected(&self) -> Result<Vec<String>> {
        Ok(self
            .lock()
            .strava
            .iter()
            .filter(|(_, l)| l.tokens.is_some())
            .map(|(p, _)| p.clone())
            .collect())
    }

    async fn set_strava_app(&self, person: &str, app: &StravaApp) -> Result<()> {
        let mut kept = self.lock();
        match kept.strava.iter_mut().find(|(p, _)| p == person) {
            Some((_, link)) => link.app = app.clone(),
            None => kept.strava.push((
                person.into(),
                StravaLink {
                    app: app.clone(),
                    tokens: None,
                    athlete: None,
                    last_sync: None,
                    last_error: None,
                },
            )),
        }
        Ok(())
    }

    async fn remove_strava(&self, person: &str) -> Result<bool> {
        let mut kept = self.lock();
        let before = kept.strava.len();
        kept.strava.retain(|(p, _)| p != person);
        Ok(kept.strava.len() != before)
    }

    async fn set_strava_tokens(
        &self,
        person: &str,
        tokens: &StravaTokens,
        athlete: Option<&Athlete>,
    ) -> Result<()> {
        self.with_link(person, |link| {
            link.tokens = Some(tokens.clone());
            if let Some(athlete) = athlete {
                link.athlete = Some(athlete.clone());
            }
        });
        Ok(())
    }

    async fn clear_strava_tokens(&self, person: &str) -> Result<()> {
        self.with_link(person, |link| {
            link.tokens = None;
            link.athlete = None;
        });
        Ok(())
    }

    async fn strava_synced(
        &self,
        person: &str,
        at: Option<DateTime<Utc>>,
        error: Option<&str>,
    ) -> Result<()> {
        self.with_link(person, |link| {
            if at.is_some() {
                link.last_sync = at;
            }
            link.last_error = error.map(str::to_string);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, started: &str) -> Activity {
        Activity {
            source: "strava".into(),
            external_id: id.into(),
            started_at: DateTime::parse_from_rfc3339(started).unwrap().to_utc(),
            kind: Kind::Run,
            name: "Morning Run".into(),
            distance_m: Some(5000.0),
            duration_s: Some(1800.0),
            moving_s: Some(1750.0),
            avg_hr: None,
            max_hr: None,
            elevation_gain_m: None,
            energy_kcal: None,
            raw: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn saving_the_same_workout_twice_changes_nothing() {
        let store = MemoryTraining::new();
        let runs = [run("1", "2026-10-14T05:00:00Z")];
        let first = store.save_activities("person:majse", &runs).await.unwrap();
        assert_eq!(
            first,
            Saved {
                added: 1,
                updated: 0
            }
        );
        let again = store.save_activities("person:majse", &runs).await.unwrap();
        assert_eq!(again, Saved::default());
        assert_eq!(store.overview("person:majse").await.unwrap().runs, 1);
    }

    #[tokio::test]
    async fn a_corrected_workout_is_updated_not_added() {
        let store = MemoryTraining::new();
        let mut runs = [run("1", "2026-10-14T05:00:00Z")];
        store.save_activities("p", &runs).await.unwrap();
        runs[0].distance_m = Some(5100.0);
        let saved = store.save_activities("p", &runs).await.unwrap();
        assert_eq!(
            saved,
            Saved {
                added: 0,
                updated: 1
            }
        );
    }

    #[tokio::test]
    async fn one_person_s_data_is_not_another_s() {
        let store = MemoryTraining::new();
        let runs = [run("1", "2026-10-14T05:00:00Z")];
        store.save_activities("person:majse", &runs).await.unwrap();
        let theirs = store
            .activities("person:mark", None, None, None)
            .await
            .unwrap();
        assert!(theirs.is_empty());
        assert_eq!(store.latest("person:mark", "strava").await.unwrap(), None);
        // The same workout id for somebody else is their own row.
        let saved = store.save_activities("person:mark", &runs).await.unwrap();
        assert_eq!(saved.added, 1);
    }

    #[tokio::test]
    async fn the_latest_is_the_newest_start() {
        let store = MemoryTraining::new();
        store
            .save_activities(
                "p",
                &[
                    run("2", "2026-10-14T05:00:00Z"),
                    run("1", "2026-10-10T05:00:00Z"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            store.latest("p", "strava").await.unwrap(),
            Some(
                DateTime::parse_from_rfc3339("2026-10-14T05:00:00Z")
                    .unwrap()
                    .to_utc()
            )
        );
        assert_eq!(store.latest("p", "elsewhere").await.unwrap(), None);
    }

    #[tokio::test]
    async fn disconnecting_keeps_the_app_and_removing_keeps_the_workouts() {
        let store = MemoryTraining::new();
        let app = StravaApp {
            client_id: "1".into(),
            client_secret: "s".into(),
        };
        store.set_strava_app("p", &app).await.unwrap();
        let tokens = StravaTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: Utc::now(),
        };
        let athlete = Athlete {
            id: 7,
            name: "Majse".into(),
        };
        store
            .set_strava_tokens("p", &tokens, Some(&athlete))
            .await
            .unwrap();
        assert_eq!(store.strava_connected().await.unwrap(), ["p"]);

        store.clear_strava_tokens("p").await.unwrap();
        let link = store.strava("p").await.unwrap().unwrap();
        assert_eq!((link.app, link.tokens, link.athlete), (app, None, None));
        assert!(store.strava_connected().await.unwrap().is_empty());

        store
            .save_activities("p", &[run("1", "2026-10-14T05:00:00Z")])
            .await
            .unwrap();
        assert!(store.remove_strava("p").await.unwrap());
        assert!(!store.remove_strava("p").await.unwrap());
        assert_eq!(store.overview("p").await.unwrap().activities, 1);
    }
}

//! Each person's Strava, brought in: a backfill when they connect, then
//! a poll every [`POLL`], and "Sync now" from their profile.
//!
//! A poll rather than Strava's webhooks: a webhook needs Niles to be
//! reachable from the internet, and each app has one subscription —
//! with one app per person, that is a public callback per person for
//! what a request every quarter of an hour does as well.
//!
//! Each person has their own app, so each has their own request budget
//! (100 reads per 15 minutes, 1,000 a day by default). A sync stops
//! with a few requests to spare and picks up where it left off on the
//! next poll: activities are fetched oldest first and each is saved as
//! it comes, so the newest kept is always where to carry on from.

use crate::error::{Error, Result};
use crate::model::{Activity, Athlete, StravaLink};
use crate::store::TrainingStore;
use crate::strava::{self, Client, PER_PAGE, SOURCE};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// How far back a new connection reaches.
pub const BACKFILL: Duration = Duration::days(180);
/// How often every connected person is synced.
pub const POLL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// How far before the newest kept workout a sync asks from. A watch
/// that syncs a day late puts an older workout on Strava after a newer
/// one; asking from a little earlier catches it, for no extra requests
/// — one page holds 200 — and the ones already kept are not fetched.
pub const OVERLAP: Duration = Duration::days(3);
/// Strava gives a new access token only once the old one has under an
/// hour left, so refreshing sooner would get the same one back.
const REFRESH_AHEAD: Duration = Duration::hours(1);
/// Requests left alone in each window, so a sync never runs the app
/// into a 429 and "Disconnect" still gets through.
const RESERVE: i64 = 3;

/// What a sync did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Synced {
    pub added: usize,
    pub updated: usize,
    /// Stopped short to stay within Strava's limits; the rest follows
    /// after this.
    pub paused_until: Option<DateTime<Utc>>,
}

/// What one sync brought that was not kept before, for whoever follows
/// the runs: a training plan marks its sessions from them.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrived {
    pub person: String,
    /// Oldest first. Empty after a sync that found nothing new, which
    /// is still news: the days gone by without a run are over.
    pub activities: Vec<Activity>,
}

pub struct Strava {
    store: Arc<dyn TrainingStore>,
    client: Client,
    arrivals: Option<tokio::sync::mpsc::UnboundedSender<Arrived>>,
    /// Per person, since each has their own app: until when Strava's
    /// limits say to leave it alone.
    paused: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Who is being synced now. One sync per person at a time: two
    /// would refresh the same token, and Strava kills the refresh token
    /// the first one was handed.
    running: Arc<Mutex<HashSet<String>>>,
}

/// A person's place in `running`, given up when dropped.
pub struct Claim {
    running: Arc<Mutex<HashSet<String>>>,
    person: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.running).remove(&self.person);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Strava {
    pub fn new(store: Arc<dyn TrainingStore>, client: Client) -> Self {
        Self {
            store,
            client,
            arrivals: None,
            paused: Mutex::default(),
            running: Arc::default(),
        }
    }

    /// Tells `to` after every sync that got through, and after one that
    /// did not but kept something first.
    pub fn with_arrivals(mut self, to: tokio::sync::mpsc::UnboundedSender<Arrived>) -> Self {
        self.arrivals = Some(to);
        self
    }

    pub fn store(&self) -> &Arc<dyn TrainingStore> {
        &self.store
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn is_syncing(&self, person: &str) -> bool {
        lock(&self.running).contains(person)
    }

    pub fn paused_until(&self, person: &str) -> Option<DateTime<Utc>> {
        lock(&self.paused)
            .get(person)
            .copied()
            .filter(|until| *until > Utc::now())
    }

    fn pause(&self, person: &str, until: DateTime<Utc>) {
        lock(&self.paused).insert(person.to_string(), until);
    }

    fn claim(&self, person: &str) -> Result<Claim> {
        if !lock(&self.running).insert(person.to_string()) {
            return Err(Error::Busy);
        }
        Ok(Claim {
            running: self.running.clone(),
            person: person.to_string(),
        })
    }

    /// The code from Strava's callback, for the person's tokens. Their
    /// app must already be saved: its secret is what the code is
    /// exchanged with.
    pub async fn connect(&self, person: &str, code: &str) -> Result<Athlete> {
        let link = self.store.strava(person).await?.ok_or(Error::Invalid {
            kind: "Strava app",
            reason: "save your Strava app's Client ID and Client Secret first".into(),
        })?;
        let (tokens, athlete) = self.client.exchange(&link.app, code).await?;
        self.store
            .set_strava_tokens(person, &tokens, Some(&athlete))
            .await?;
        self.store.strava_synced(person, None, None).await?;
        Ok(athlete)
    }

    /// Takes the person's grant back at Strava, then forgets the tokens.
    /// They are forgotten whether or not Strava could be told: the
    /// person asked for Niles to stop reading, and it does. False when
    /// Strava could not be told, so the card can say where to finish it.
    pub async fn disconnect(&self, person: &str) -> Result<bool> {
        let link = self
            .store
            .strava(person)
            .await?
            .ok_or(Error::NotConnected)?;
        let Some(tokens) = &link.tokens else {
            return Err(Error::NotConnected);
        };
        let told = match self.client.revoke(&link.app, &tokens.refresh_token).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("[strava] {person}: revoking at Strava failed: {e}");
                false
            }
        };
        self.store.clear_strava_tokens(person).await?;
        self.store.strava_synced(person, None, None).await?;
        Ok(told)
    }

    /// Syncs in the background, or says at once why it will not.
    pub fn start(self: &Arc<Self>, person: &str) -> Result<()> {
        if let Some(until) = self.paused_until(person) {
            return Err(Error::RateLimited { until });
        }
        let claim = self.claim(person)?;
        let this = self.clone();
        let person = person.to_string();
        tokio::spawn(async move {
            match this.sync_claimed(&person, claim).await {
                Ok(s) => tracing::info!(
                    "[strava] {person}: {} new, {} changed{}",
                    s.added,
                    s.updated,
                    s.paused_until
                        .map(|t| format!(", the rest after {t}"))
                        .unwrap_or_default()
                ),
                Err(e) => tracing::warn!("[strava] {person}: sync failed: {e}"),
            }
        });
        Ok(())
    }

    pub async fn sync(&self, person: &str) -> Result<Synced> {
        let claim = self.claim(person)?;
        self.sync_claimed(person, claim).await
    }

    /// Every connected person, one after another.
    pub async fn sync_all(&self) {
        let people = match self.store.strava_connected().await {
            Ok(people) => people,
            Err(e) => {
                tracing::warn!("[strava] could not read who is connected: {e}");
                return;
            }
        };
        for person in people {
            if self.paused_until(&person).is_some() {
                continue;
            }
            match self.sync(&person).await {
                Ok(s) if s.added + s.updated > 0 => {
                    tracing::info!("[strava] {person}: {} new, {} changed", s.added, s.updated)
                }
                Ok(_) | Err(Error::Busy) => {}
                Err(e) => tracing::warn!("[strava] {person}: sync failed: {e}"),
            }
        }
    }

    /// The poll, forever. The first round is at once, so a backfill a
    /// restart cut short carries on.
    pub async fn poll(self: Arc<Self>) {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            self.sync_all().await;
        }
    }

    async fn sync_claimed(&self, person: &str, _claim: Claim) -> Result<Synced> {
        if let Some(until) = self.paused_until(person) {
            return Err(Error::RateLimited { until });
        }
        let now = Utc::now();
        let mut new = Vec::new();
        let outcome = self.fetch(person, now, &mut new).await;
        if let Some(to) = &self.arrivals
            && (outcome.is_ok() || !new.is_empty())
        {
            // Nobody listening any more is nobody to tell.
            let _ = to.send(Arrived {
                person: person.to_string(),
                activities: new,
            });
        }
        let said = match &outcome {
            Ok(synced) => {
                if let Some(until) = synced.paused_until {
                    self.pause(person, until);
                }
                return self
                    .store
                    .strava_synced(person, Some(now), None)
                    .await
                    .and(outcome);
            }
            Err(Error::NotConnected) => return outcome,
            Err(Error::RateLimited { until }) => {
                self.pause(person, *until);
                outcome.as_ref().err().map(ToString::to_string)
            }
            Err(e @ Error::Unauthorized) => {
                // Revoked on strava.com: nothing to read with any more,
                // and the poll should stop asking.
                self.store.clear_strava_tokens(person).await?;
                Some(e.to_string())
            }
            Err(e) => Some(e.to_string()),
        };
        self.store
            .strava_synced(person, None, said.as_deref())
            .await?;
        outcome
    }

    /// An access token with at least an hour left: the kept one, or a
    /// refreshed one, kept before it is used — with the refresh token
    /// that came with it, which may be new.
    async fn access_token(
        &self,
        person: &str,
        link: &StravaLink,
        now: DateTime<Utc>,
    ) -> Result<String> {
        let tokens = link.tokens.as_ref().ok_or(Error::NotConnected)?;
        if tokens.expires_at - now > REFRESH_AHEAD {
            return Ok(tokens.access_token.clone());
        }
        let fresh = self
            .client
            .refresh(&link.app, &tokens.refresh_token)
            .await?;
        self.store.set_strava_tokens(person, &fresh, None).await?;
        Ok(fresh.access_token)
    }

    /// Each workout not kept before goes into `new` as it is saved, so
    /// what a sync cut short did keep is still known.
    async fn fetch(
        &self,
        person: &str,
        now: DateTime<Utc>,
        new: &mut Vec<Activity>,
    ) -> Result<Synced> {
        let link = self
            .store
            .strava(person)
            .await?
            .ok_or(Error::NotConnected)?;
        let token = self.access_token(person, &link, now).await?;
        let after = match self.store.latest(person, SOURCE).await? {
            Some(newest) => newest - OVERLAP,
            None => now - BACKFILL,
        };
        let known: HashSet<String> = self
            .store
            .activities(person, Some(after), None, None)
            .await?
            .into_iter()
            .filter(|a| a.source == SOURCE)
            .map(|a| a.external_id)
            .collect();

        // The whole list before any detail. Whether Strava sorts it
        // ascending or descending, a partial list fetched in detail
        // could leave a gap behind the newest kept, which the next
        // `after` would then step over.
        let mut listed = Vec::new();
        for page in 1.. {
            let (items, budget) = self.client.list(&token, after, page).await?;
            let more = items.len() >= PER_PAGE;
            listed.extend(items);
            if !more {
                break;
            }
            if let Some(until) = budget.exhausted_until(RESERVE, Utc::now()) {
                return Err(Error::RateLimited { until });
            }
        }

        let mut todo: Vec<(DateTime<Utc>, String)> = listed
            .iter()
            .filter_map(|summary| {
                let id = match summary.get("id")? {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => return None,
                };
                let started = summary.get("start_date")?.as_str()?;
                let started = DateTime::parse_from_rfc3339(started).ok()?.to_utc();
                (!known.contains(&id)).then_some((started, id))
            })
            .collect();
        todo.sort();

        let mut synced = Synced::default();
        for (i, (_, id)) in todo.iter().enumerate() {
            let (detail, budget) = match self.client.detail(&token, id).await {
                Ok(answer) => answer,
                Err(Error::RateLimited { until }) => {
                    synced.paused_until = Some(until);
                    break;
                }
                // Deleted on Strava between the list and now.
                Err(Error::Strava {
                    status: Some(404), ..
                }) => continue,
                Err(e) => return Err(e),
            };
            if let Some(activity) = strava::activity(&detail) {
                let saved = self
                    .store
                    .save_activities(person, std::slice::from_ref(&activity))
                    .await?;
                synced.added += saved.added;
                synced.updated += saved.updated;
                if saved.added > 0 {
                    new.push(activity);
                }
            }
            if i + 1 < todo.len()
                && let Some(until) = budget.exhausted_until(RESERVE, Utc::now())
            {
                synced.paused_until = Some(until);
                break;
            }
        }
        Ok(synced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{StravaApp, StravaTokens};
    use crate::store::MemoryTraining;
    use serde_json::json;
    use wiremock::matchers::{
        basic_auth, bearer_token, body_string_contains, method, path, path_regex, query_param,
    };
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const MAJSE: &str = "person:majse";
    const MARK: &str = "person:mark";

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn app() -> StravaApp {
        StravaApp {
            client_id: "1001".into(),
            client_secret: "her-secret".into(),
        }
    }

    fn tokens(expires_in: Duration) -> StravaTokens {
        StravaTokens {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: Utc::now() + expires_in,
        }
    }

    async fn connected(
        server: &MockServer,
        expires_in: Duration,
    ) -> (Arc<Strava>, Arc<MemoryTraining>) {
        let store = Arc::new(MemoryTraining::new());
        store.set_strava_app(MAJSE, &app()).await.unwrap();
        store
            .set_strava_tokens(MAJSE, &tokens(expires_in), None)
            .await
            .unwrap();
        let strava = Arc::new(Strava::new(
            store.clone(),
            Client::with_base(reqwest::Client::new(), &server.uri()),
        ));
        (strava, store)
    }

    fn summary(id: i64, started: DateTime<Utc>) -> Value {
        json!({ "id": id, "start_date": started.to_rfc3339(), "sport_type": "Run" })
    }

    /// A run for whatever id is asked for, started `id` hours after the
    /// start of the backfill.
    fn detail_for(request: &Request) -> ResponseTemplate {
        let id: i64 = request
            .url
            .path()
            .rsplit('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        ResponseTemplate::new(200).set_body_json(json!({
            "id": id,
            "name": format!("Run {id}"),
            "sport_type": "Run",
            "start_date": (Utc::now() - BACKFILL + Duration::hours(id)).to_rfc3339(),
            "distance": 5000.0,
            "moving_time": 1500,
            "elapsed_time": 1560,
            "total_elevation_gain": 10.0,
            "splits_metric": []
        }))
    }

    fn after_of(request: &Request) -> i64 {
        request
            .url
            .query_pairs()
            .find(|(k, _)| k == "after")
            .map(|(_, v)| v.parse().unwrap())
            .unwrap()
    }

    #[tokio::test]
    async fn a_new_connection_backfills_half_a_year_page_by_page() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        let start = Utc::now() - BACKFILL;
        let page = |from: i64, n: i64| {
            (from..from + n)
                .map(|id| summary(id, start + Duration::hours(id)))
                .collect::<Vec<_>>()
        };
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(query_param("page", "1"))
            .and(query_param("per_page", "200"))
            .and(bearer_token("old-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(1, 200)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(201, 5)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v3/activities/\d+$"))
            .and(query_param("include_all_efforts", "false"))
            .respond_with(detail_for)
            .expect(205)
            .mount(&server)
            .await;

        let synced = strava.sync(MAJSE).await.unwrap();
        assert_eq!((synced.added, synced.paused_until), (205, None));
        assert_eq!(store.overview(MAJSE).await.unwrap().runs, 205);

        let asked = server.received_requests().await.unwrap();
        let after = after_of(&asked[0]);
        assert!(
            (after - start.timestamp()).abs() < 60,
            "after={after}, 180 days ago={}",
            start.timestamp()
        );
        let link = store.strava(MAJSE).await.unwrap().unwrap();
        assert!(link.last_sync.is_some());
        assert_eq!(link.last_error, None);
    }

    #[tokio::test]
    async fn the_poll_asks_from_the_newest_kept_and_fetches_only_what_is_new() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        let kept = at("2026-10-10T06:00:00Z");
        let mut first = crate::strava::activity(&json!({
            "id": 1, "start_date": kept.to_rfc3339(), "sport_type": "Run"
        }))
        .unwrap();
        first.name = "kept".into();
        store.save_activities(MAJSE, &[first]).await.unwrap();

        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(query_param(
                "after",
                (kept - OVERLAP).timestamp().to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                summary(1, kept),
                summary(2, kept + Duration::days(1)),
            ])))
            .expect(1)
            .mount(&server)
            .await;
        // The second time, from the one it fetched.
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(query_param(
                "after",
                (kept + Duration::days(1) - OVERLAP).timestamp().to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                summary(1, kept),
                summary(2, kept + Duration::days(1)),
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 2, "sport_type": "Run",
                "start_date": (kept + Duration::days(1)).to_rfc3339()
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/1"))
            .respond_with(detail_for)
            .expect(0)
            .mount(&server)
            .await;

        let synced = strava.sync(MAJSE).await.unwrap();
        assert_eq!(synced.added, 1);
        // Again, with nothing new: the second listing finds both kept.
        let again = strava.sync(MAJSE).await.unwrap();
        assert_eq!(again, Synced::default());
    }

    #[tokio::test]
    async fn a_429_pauses_until_the_window_turns_and_keeps_what_came() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        let start = Utc::now() - BACKFILL;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                summary(1, start + Duration::hours(1)),
                summary(2, start + Duration::hours(2)),
                summary(3, start + Duration::hours(3)),
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/1"))
            .respond_with(detail_for)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/2"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("x-ratelimit-limit", "200,2000")
                    .insert_header("x-ratelimit-usage", "200,700")
                    .set_body_json(json!({ "message": "Rate Limit Exceeded" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let synced = strava.sync(MAJSE).await.unwrap();
        assert_eq!(synced.added, 1, "the first is kept");
        let until = synced.paused_until.expect("paused");
        assert!(until > Utc::now() && until <= Utc::now() + Duration::minutes(15));
        assert_eq!(store.overview(MAJSE).await.unwrap().activities, 1);

        // Asked again before the window turns: no request goes out.
        assert!(matches!(
            strava.sync(MAJSE).await,
            Err(Error::RateLimited { .. })
        ));
        assert!(matches!(
            strava.start(MAJSE),
            Err(Error::RateLimited { .. })
        ));
    }

    #[tokio::test]
    async fn a_budget_nearly_spent_stops_before_strava_has_to_say_no() {
        let server = MockServer::start().await;
        let (strava, _) = connected(&server, Duration::hours(5)).await;
        let start = Utc::now() - BACKFILL;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                summary(1, start + Duration::hours(1)),
                summary(2, start + Duration::hours(2)),
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/1"))
            .respond_with(move |r: &Request| {
                detail_for(r)
                    .insert_header("x-readratelimit-limit", "100,1000")
                    .insert_header("x-readratelimit-usage", "98,900")
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/activities/2"))
            .respond_with(detail_for)
            .expect(0)
            .mount(&server)
            .await;
        let synced = strava.sync(MAJSE).await.unwrap();
        assert_eq!(synced.added, 1);
        assert!(synced.paused_until.is_some());
    }

    #[tokio::test]
    async fn a_token_near_its_end_is_refreshed_and_the_rotated_one_kept() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::minutes(30)).await;
        let expires = Utc::now() + Duration::hours(6);
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .and(body_string_contains("client_id=1001"))
            .and(body_string_contains("client_secret=her-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "token_type": "Bearer",
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "expires_at": expires.timestamp(),
                "expires_in": 21600
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(bearer_token("new-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;

        strava.sync(MAJSE).await.unwrap();
        let kept = store.strava(MAJSE).await.unwrap().unwrap().tokens.unwrap();
        assert_eq!(kept.access_token, "new-access");
        assert_eq!(kept.refresh_token, "new-refresh");
        assert_eq!(kept.expires_at.timestamp(), expires.timestamp());
    }

    #[tokio::test]
    async fn a_token_with_hours_left_is_used_as_it_is() {
        let server = MockServer::start().await;
        let (strava, _) = connected(&server, Duration::hours(5)).await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .and(bearer_token("old-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;
        strava.sync(MAJSE).await.unwrap();
    }

    #[tokio::test]
    async fn a_connection_revoked_on_strava_is_forgotten_and_said() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert!(matches!(strava.sync(MAJSE).await, Err(Error::Unauthorized)));
        let link = store.strava(MAJSE).await.unwrap().unwrap();
        assert!(link.tokens.is_none());
        assert_eq!(link.app, app(), "the app stays");
        assert!(link.last_error.unwrap().contains("connect again"));
        assert!(store.strava_connected().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn disconnecting_revokes_at_strava_then_forgets_the_tokens() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .and(basic_auth("1001", "her-secret"))
            .and(body_string_contains("token=old-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        assert!(strava.disconnect(MAJSE).await.unwrap());
        let link = store.strava(MAJSE).await.unwrap().unwrap();
        assert!(link.tokens.is_none());
        assert_eq!(link.app, app());
        assert!(matches!(
            strava.disconnect(MAJSE).await,
            Err(Error::NotConnected)
        ));
    }

    #[tokio::test]
    async fn disconnecting_forgets_the_tokens_even_when_strava_is_down() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        assert!(!strava.disconnect(MAJSE).await.unwrap());
        assert!(store.strava(MAJSE).await.unwrap().unwrap().tokens.is_none());
    }

    #[tokio::test]
    async fn connecting_keeps_the_tokens_and_athlete_under_whoever_connected() {
        let server = MockServer::start().await;
        let store = Arc::new(MemoryTraining::new());
        store.set_strava_app(MAJSE, &app()).await.unwrap();
        let strava = Strava::new(
            store.clone(),
            Client::with_base(reqwest::Client::new(), &server.uri()),
        );
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "a", "refresh_token": "r",
                "expires_at": (Utc::now() + Duration::hours(6)).timestamp(),
                "athlete": { "id": 42, "firstname": "Majse", "lastname": "H" }
            })))
            .mount(&server)
            .await;

        let athlete = strava.connect(MAJSE, "the-code").await.unwrap();
        assert_eq!(athlete.name, "Majse H");
        assert_eq!(store.strava_connected().await.unwrap(), [MAJSE]);
        assert!(store.strava(MARK).await.unwrap().is_none());
        // Mark has no app of his own to exchange a code with.
        assert!(matches!(
            strava.connect(MARK, "the-code").await,
            Err(Error::Invalid { .. })
        ));
    }

    #[tokio::test]
    async fn one_person_s_sync_lands_with_them_alone() {
        let server = MockServer::start().await;
        let (strava, store) = connected(&server, Duration::hours(5)).await;
        store
            .set_strava_app(
                MARK,
                &StravaApp {
                    client_id: "2002".into(),
                    client_secret: "his".into(),
                },
            )
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([summary(5, Utc::now() - Duration::days(1))])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v3/activities/\d+$"))
            .respond_with(detail_for)
            .mount(&server)
            .await;
        strava.sync_all().await;
        assert_eq!(store.overview(MAJSE).await.unwrap().activities, 1);
        assert_eq!(store.overview(MARK).await.unwrap().activities, 0);
        assert!(matches!(strava.sync(MARK).await, Err(Error::NotConnected)));
    }

    #[tokio::test]
    async fn each_sync_tells_who_follows_the_runs_what_was_new() {
        let server = MockServer::start().await;
        let (_, store) = connected(&server, Duration::hours(5)).await;
        let (to, mut arrived) = tokio::sync::mpsc::unbounded_channel();
        let strava = Strava::new(
            store.clone(),
            Client::with_base(reqwest::Client::new(), &server.uri()),
        )
        .with_arrivals(to);
        let start = Utc::now() - BACKFILL;
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                summary(1, start + Duration::hours(1)),
                summary(2, start + Duration::hours(2)),
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v3/activities/\d+$"))
            .respond_with(detail_for)
            .mount(&server)
            .await;

        strava.sync(MAJSE).await.unwrap();
        let first = arrived.try_recv().unwrap();
        assert_eq!(first.person, MAJSE);
        let ids: Vec<&str> = first
            .activities
            .iter()
            .map(|a| a.external_id.as_str())
            .collect();
        assert_eq!(ids, ["1", "2"]);

        // Nothing new is still said: the days without a run are over.
        strava.sync(MAJSE).await.unwrap();
        assert_eq!(arrived.try_recv().unwrap().activities, vec![]);
        assert!(arrived.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_sync_that_kept_nothing_and_failed_tells_nobody() {
        let server = MockServer::start().await;
        let (_, store) = connected(&server, Duration::hours(5)).await;
        let (to, mut arrived) = tokio::sync::mpsc::unbounded_channel();
        let strava = Strava::new(
            store,
            Client::with_base(reqwest::Client::new(), &server.uri()),
        )
        .with_arrivals(to);
        Mock::given(method("GET"))
            .and(path("/api/v3/athlete/activities"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        assert!(strava.sync(MAJSE).await.is_err());
        assert!(arrived.try_recv().is_err());
    }

    #[tokio::test]
    async fn one_sync_at_a_time_per_person() {
        let server = MockServer::start().await;
        let (strava, _) = connected(&server, Duration::hours(5)).await;
        let claim = strava.claim(MAJSE).unwrap();
        assert!(strava.is_syncing(MAJSE));
        assert!(matches!(strava.sync(MAJSE).await, Err(Error::Busy)));
        drop(claim);
        assert!(!strava.is_syncing(MAJSE));
    }
}

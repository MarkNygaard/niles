//! A person's calendars, as the voice tools and the app use them.
//!
//! Everything here is asked *for a person*: their connections are the
//! only ones looked at, and a reference to an event only means anything
//! to the person whose list it came from.

use crate::error::{Error, Result};
use crate::graph::{self, MicrosoftApp, Tokens};
use crate::model::{Access, Connection, Event, NewEvent};
use crate::store::ConnectionStore;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What Microsoft is asked, behind a seam so the rules above it can be
/// tested without a network.
#[async_trait]
pub trait Backend: Send + Sync {
    async fn refresh(&self, app: &MicrosoftApp, connection: &Connection) -> Result<Tokens>;
    async fn events(
        &self,
        token: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        tz: Tz,
    ) -> Result<Vec<Event>>;
    async fn create(&self, token: &str, event: &NewEvent, tz: Tz) -> Result<Event>;
    async fn update(&self, token: &str, id: &str, event: &NewEvent, tz: Tz) -> Result<Event>;
    async fn delete(&self, token: &str, id: &str) -> Result<()>;
}

/// Microsoft Graph itself.
pub struct Graph {
    http: reqwest::Client,
}

impl Graph {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Backend for Graph {
    async fn refresh(&self, app: &MicrosoftApp, connection: &Connection) -> Result<Tokens> {
        app.refresh(
            &self.http,
            &connection.refresh_token,
            connection.access,
            &connection.name,
        )
        .await
    }
    async fn events(
        &self,
        token: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        tz: Tz,
    ) -> Result<Vec<Event>> {
        graph::events(&self.http, token, from, to, tz).await
    }
    async fn create(&self, token: &str, event: &NewEvent, tz: Tz) -> Result<Event> {
        graph::create(&self.http, token, event, tz).await
    }
    async fn update(&self, token: &str, id: &str, event: &NewEvent, tz: Tz) -> Result<Event> {
        graph::update(&self.http, token, id, event, tz).await
    }
    async fn delete(&self, token: &str, id: &str) -> Result<()> {
        graph::delete(&self.http, token, id).await
    }
}

/// An event as the tools hand it on: with the calendar it is in and a
/// short reference to change or remove it by. Graph's own ids run to a
/// hundred and fifty characters, which a model copies back wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub reference: String,
    pub calendar: String,
    pub event: Event,
}

/// What an event is to become; anything left out stays as it was.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub subject: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub end: Option<NaiveDateTime>,
    pub all_day: Option<bool>,
    pub location: Option<String>,
}

struct Known {
    reference: String,
    connection: String,
    calendar: String,
    event: Event,
}

pub struct Calendars {
    store: Arc<dyn ConnectionStore>,
    backend: Arc<dyn Backend>,
    /// Access tokens by connection, until shortly before they expire.
    /// In memory only: an hour's worth of access is not worth storing.
    tokens: tokio::sync::Mutex<HashMap<String, (String, DateTime<Utc>)>>,
    /// The events each person was last shown, by reference.
    known: Mutex<HashMap<String, Vec<Known>>>,
}

impl Calendars {
    pub fn new(store: Arc<dyn ConnectionStore>, backend: Arc<dyn Backend>) -> Self {
        Self {
            store,
            backend,
            tokens: tokio::sync::Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
        }
    }

    pub async fn connections(&self, person: &str) -> Result<Vec<Connection>> {
        self.store.list(person).await
    }

    /// Keep a connection a sign-in just made. `replace` is the one it
    /// takes the place of — the same account connected again with
    /// different access — and its id is kept.
    pub async fn connect(
        &self,
        person: &str,
        name: &str,
        access: Access,
        account: &str,
        tokens: Tokens,
        replace: Option<&str>,
    ) -> Result<Connection> {
        let existing = self.store.list(person).await?;
        let id = match replace.filter(|r| existing.iter().any(|c| c.id == *r)) {
            Some(id) => id.to_string(),
            None => new_id(),
        };
        let connection = Connection {
            id: id.clone(),
            person: person.to_string(),
            name: name.trim().to_string(),
            account: account.to_string(),
            access,
            refresh_token: tokens.refresh_token.clone(),
        };
        self.store.save(&connection).await?;
        self.tokens
            .lock()
            .await
            .insert(id, (tokens.access_token, tokens.expires_at));
        Ok(connection)
    }

    pub async fn disconnect(&self, person: &str, id: &str) -> Result<bool> {
        self.tokens.lock().await.remove(id);
        self.store.delete(person, id).await
    }

    /// Every event of `person`'s in the window, all their calendars
    /// together, earliest first. A calendar that fails is said in the
    /// second list rather than hiding the others.
    pub async fn events(
        &self,
        app: &MicrosoftApp,
        person: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        tz: Tz,
    ) -> Result<(Vec<Listed>, Vec<String>)> {
        let mut found: Vec<(Connection, Event)> = Vec::new();
        let mut problems = Vec::new();
        for connection in self.store.list(person).await? {
            let events = match self.token(app, &connection).await {
                Ok(token) => self.backend.events(&token, from, to, tz).await,
                Err(e) => Err(e),
            };
            match events {
                Ok(events) => found.extend(events.into_iter().map(|e| (connection.clone(), e))),
                Err(e) => problems.push(format!("{}: {e}", connection.name)),
            }
        }
        found.sort_by_key(|(_, e)| (e.start, !e.all_day));
        let listed: Vec<Known> = found
            .into_iter()
            .enumerate()
            .map(|(i, (connection, event))| Known {
                reference: format!("e{}", i + 1),
                connection: connection.id,
                calendar: connection.name,
                event,
            })
            .collect();
        let out = listed.iter().map(listed_of).collect();
        self.lock_known().insert(person.to_string(), listed);
        Ok((out, problems))
    }

    /// Make an event in the calendar named, or in the only one `person`
    /// can write to.
    pub async fn create(
        &self,
        app: &MicrosoftApp,
        person: &str,
        calendar: Option<&str>,
        event: &NewEvent,
        tz: Tz,
    ) -> Result<Listed> {
        let connection = self.writable(person, calendar).await?;
        let token = self.token(app, &connection).await?;
        let made = self.backend.create(&token, event, tz).await?;
        Ok(self.remember(person, &connection, made))
    }

    pub async fn update(
        &self,
        app: &MicrosoftApp,
        person: &str,
        reference: &str,
        change: &Change,
        tz: Tz,
    ) -> Result<Listed> {
        let (connection, before) = self.known_event(person, reference).await?;
        writable_or_refuse(&connection)?;
        let token = self.token(app, &connection).await?;
        let changed = self
            .backend
            .update(&token, &before.id, &apply(&before, change), tz)
            .await?;
        let mut known = self.lock_known();
        if let Some(entry) = known
            .get_mut(person)
            .and_then(|all| all.iter_mut().find(|k| k.reference == reference))
        {
            entry.event = changed.clone();
        }
        Ok(Listed {
            reference: reference.to_string(),
            calendar: connection.name,
            event: changed,
        })
    }

    pub async fn delete(
        &self,
        app: &MicrosoftApp,
        person: &str,
        reference: &str,
    ) -> Result<Listed> {
        let (connection, event) = self.known_event(person, reference).await?;
        writable_or_refuse(&connection)?;
        let token = self.token(app, &connection).await?;
        self.backend.delete(&token, &event.id).await?;
        if let Some(all) = self.lock_known().get_mut(person) {
            all.retain(|k| k.reference != reference);
        }
        Ok(Listed {
            reference: reference.to_string(),
            calendar: connection.name,
            event,
        })
    }

    async fn writable(&self, person: &str, calendar: Option<&str>) -> Result<Connection> {
        let all = self.store.list(person).await?;
        if let Some(name) = calendar {
            let connection = all
                .into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
                .ok_or_else(|| Error::NoSuchConnection {
                    name: name.to_string(),
                })?;
            writable_or_refuse(&connection)?;
            return Ok(connection);
        }
        let mut writable: Vec<Connection> = all
            .into_iter()
            .filter(|c| c.access == Access::Write)
            .collect();
        match writable.len() {
            0 => Err(Error::Invalid {
                reason: "none of your calendars is connected with write access".into(),
            }),
            1 => Ok(writable.remove(0)),
            _ => Err(Error::Invalid {
                reason: format!(
                    "which calendar: {}?",
                    writable
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
            }),
        }
    }

    async fn known_event(&self, person: &str, reference: &str) -> Result<(Connection, Event)> {
        let (connection_id, event) = self
            .lock_known()
            .get(person)
            .and_then(|all| all.iter().find(|k| k.reference == reference.trim()))
            .map(|k| (k.connection.clone(), k.event.clone()))
            .ok_or_else(|| Error::NoSuchEvent {
                reference: reference.to_string(),
            })?;
        // Looked up again, so a connection removed since the list was
        // read is not written through.
        let connection = self
            .store
            .list(person)
            .await?
            .into_iter()
            .find(|c| c.id == connection_id)
            .ok_or_else(|| Error::NoSuchEvent {
                reference: reference.to_string(),
            })?;
        Ok((connection, event))
    }

    fn remember(&self, person: &str, connection: &Connection, event: Event) -> Listed {
        let mut known = self.lock_known();
        let all = known.entry(person.to_string()).or_default();
        let reference = format!("e{}", all.len() + 1);
        let entry = Known {
            reference,
            connection: connection.id.clone(),
            calendar: connection.name.clone(),
            event,
        };
        let out = listed_of(&entry);
        all.push(entry);
        out
    }

    /// A live access token for the connection, refreshed when it has
    /// less than a minute left. A rotated refresh token is kept.
    async fn token(&self, app: &MicrosoftApp, connection: &Connection) -> Result<String> {
        let mut tokens = self.tokens.lock().await;
        if let Some((token, until)) = tokens.get(&connection.id)
            && *until - chrono::Duration::seconds(60) > Utc::now()
        {
            return Ok(token.clone());
        }
        let fresh = self.backend.refresh(app, connection).await?;
        if fresh.refresh_token != connection.refresh_token {
            self.store
                .update_token(&connection.id, &fresh.refresh_token)
                .await?;
        }
        tokens.insert(
            connection.id.clone(),
            (fresh.access_token.clone(), fresh.expires_at),
        );
        Ok(fresh.access_token)
    }

    fn lock_known(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Known>>> {
        self.known.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn listed_of(known: &Known) -> Listed {
    Listed {
        reference: known.reference.clone(),
        calendar: known.calendar.clone(),
        event: known.event.clone(),
    }
}

fn writable_or_refuse(connection: &Connection) -> Result<()> {
    if connection.access == Access::Write {
        Ok(())
    } else {
        Err(Error::ReadOnly {
            name: connection.name.clone(),
        })
    }
}

/// The event after a change. A new start alone keeps the length it had,
/// so "move it to eleven" does not leave it ending before it starts.
fn apply(before: &Event, change: &Change) -> NewEvent {
    let start = change.start.unwrap_or(before.start);
    let end = change
        .end
        .unwrap_or_else(|| start + (before.end - before.start));
    NewEvent {
        subject: change
            .subject
            .clone()
            .unwrap_or_else(|| before.subject.clone()),
        start,
        end,
        all_day: change.all_day.unwrap_or(before.all_day),
        location: change.location.clone().or_else(|| before.location.clone()),
    }
}

fn new_id() -> String {
    format!("cal-{:016x}", rand::random::<u64>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryConnections;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        refreshes: AtomicUsize,
        writes: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Backend for Fake {
        async fn refresh(&self, _app: &MicrosoftApp, connection: &Connection) -> Result<Tokens> {
            if connection.refresh_token == "revoked" {
                return Err(Error::Reconnect {
                    name: connection.name.clone(),
                });
            }
            let n = self.refreshes.fetch_add(1, Ordering::SeqCst);
            Ok(Tokens {
                access_token: format!("access-{}", connection.name),
                refresh_token: format!("rotated-{n}"),
                expires_at: Utc::now() + chrono::Duration::hours(1),
            })
        }
        async fn events(
            &self,
            token: &str,
            _from: DateTime<Utc>,
            _to: DateTime<Utc>,
            _tz: Tz,
        ) -> Result<Vec<Event>> {
            let (subject, hour) = if token.ends_with("Work") {
                ("Standup", 9)
            } else {
                ("Dentist", 8)
            };
            Ok(vec![event(subject, hour)])
        }
        async fn create(&self, token: &str, event: &NewEvent, _tz: Tz) -> Result<Event> {
            self.writes
                .lock()
                .unwrap()
                .push(format!("create {} via {token}", event.subject));
            Ok(Event {
                id: "new".into(),
                subject: event.subject.clone(),
                start: event.start,
                end: event.end,
                all_day: event.all_day,
                location: event.location.clone(),
            })
        }
        async fn update(&self, _token: &str, id: &str, event: &NewEvent, _tz: Tz) -> Result<Event> {
            self.writes
                .lock()
                .unwrap()
                .push(format!("update {id} to {}", event.start));
            Ok(Event {
                id: id.into(),
                subject: event.subject.clone(),
                start: event.start,
                end: event.end,
                all_day: event.all_day,
                location: event.location.clone(),
            })
        }
        async fn delete(&self, _token: &str, id: &str) -> Result<()> {
            self.writes.lock().unwrap().push(format!("delete {id}"));
            Ok(())
        }
    }

    fn event(subject: &str, hour: u32) -> Event {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        Event {
            id: format!("graph-{subject}"),
            subject: subject.into(),
            start: day.and_hms_opt(hour, 0, 0).unwrap(),
            end: day.and_hms_opt(hour + 1, 0, 0).unwrap(),
            all_day: false,
            location: None,
        }
    }

    fn app() -> MicrosoftApp {
        MicrosoftApp {
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://x/cb".into(),
        }
    }

    fn tz() -> Tz {
        "Europe/Copenhagen".parse().unwrap()
    }

    fn connection(id: &str, person: &str, name: &str, access: Access, token: &str) -> Connection {
        Connection {
            id: id.into(),
            person: person.into(),
            name: name.into(),
            account: format!("{name}@example.com"),
            access,
            refresh_token: token.into(),
        }
    }

    /// Mark: a private calendar he can write, a work one he can read.
    /// Majse: one of her own.
    async fn household() -> (Calendars, Arc<Fake>, Arc<MemoryConnections>) {
        let store = Arc::new(MemoryConnections::new());
        for c in [
            connection("p", "mark", "Private", Access::Write, "rt-p"),
            connection("w", "mark", "Work", Access::Read, "rt-w"),
            connection("m", "majse", "Majse", Access::Write, "rt-m"),
        ] {
            store.save(&c).await.unwrap();
        }
        let fake = Arc::new(Fake::default());
        (Calendars::new(store.clone(), fake.clone()), fake, store)
    }

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (Utc::now(), Utc::now() + chrono::Duration::days(1))
    }

    #[tokio::test]
    async fn reads_every_calendar_of_the_person_asking_and_no_one_else() {
        let (calendars, _, _) = household().await;
        let (from, to) = window();
        let (listed, problems) = calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        assert!(problems.is_empty());
        let seen: Vec<_> = listed
            .iter()
            .map(|l| {
                (
                    l.reference.as_str(),
                    l.calendar.as_str(),
                    l.event.subject.as_str(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [("e1", "Private", "Dentist"), ("e2", "Work", "Standup")]
        );
        // Majse's references are her own; Mark's mean nothing to her.
        assert!(matches!(
            calendars.delete(&app(), "majse", "e2").await,
            Err(Error::NoSuchEvent { .. })
        ));
    }

    #[tokio::test]
    async fn a_read_only_calendar_is_never_written() {
        let (calendars, fake, _) = household().await;
        let (from, to) = window();
        calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        assert!(matches!(
            calendars.delete(&app(), "mark", "e2").await,
            Err(Error::ReadOnly { .. })
        ));
        let lunch = NewEvent {
            subject: "Lunch".into(),
            ..apply(&event("x", 12), &Change::default())
        };
        assert!(matches!(
            calendars
                .create(&app(), "mark", Some("work"), &lunch, tz())
                .await,
            Err(Error::ReadOnly { .. })
        ));
        assert!(fake.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn writes_go_to_the_one_calendar_that_can_take_them() {
        let (calendars, fake, _) = household().await;
        let dentist = NewEvent {
            subject: "Dentist".into(),
            ..apply(&event("x", 10), &Change::default())
        };
        let made = calendars
            .create(&app(), "mark", None, &dentist, tz())
            .await
            .unwrap();
        assert_eq!(made.calendar, "Private");
        assert_eq!(
            fake.writes.lock().unwrap().as_slice(),
            ["create Dentist via access-Private"]
        );
    }

    #[tokio::test]
    async fn moving_an_event_keeps_its_length() {
        let (calendars, fake, _) = household().await;
        let (from, to) = window();
        calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        let eleven = chrono::NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(11, 0, 0)
            .unwrap();
        let moved = calendars
            .update(
                &app(),
                "mark",
                "e1",
                &Change {
                    start: Some(eleven),
                    ..Change::default()
                },
                tz(),
            )
            .await
            .unwrap();
        assert_eq!(moved.event.end, eleven + chrono::Duration::hours(1));
        assert_eq!(
            fake.writes.lock().unwrap().as_slice(),
            ["update graph-Dentist to 2026-10-12 11:00:00"]
        );
    }

    #[tokio::test]
    async fn keeps_the_rotated_token_and_reuses_the_access_token() {
        let (calendars, fake, store) = household().await;
        let (from, to) = window();
        calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        assert_eq!(
            fake.refreshes.load(Ordering::SeqCst),
            2,
            "once per calendar"
        );
        let kept = store.list("mark").await.unwrap();
        assert!(kept.iter().all(|c| c.refresh_token.starts_with("rotated-")));
    }

    #[tokio::test]
    async fn a_revoked_calendar_is_said_without_hiding_the_other() {
        let (calendars, _, store) = household().await;
        store
            .save(&connection("w", "mark", "Work", Access::Read, "revoked"))
            .await
            .unwrap();
        let (from, to) = window();
        let (listed, problems) = calendars
            .events(&app(), "mark", from, to, tz())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            problems,
            ["Work: the Work calendar has to be connected again"]
        );
    }

    #[tokio::test]
    async fn connecting_again_keeps_the_connection() {
        let (calendars, _, store) = household().await;
        let tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        let again = calendars
            .connect("mark", "Work", Access::Write, "w@x", tokens, Some("w"))
            .await
            .unwrap();
        assert_eq!(again.id, "w");
        let mark = store.list("mark").await.unwrap();
        assert_eq!(mark.len(), 2);
        assert_eq!(mark[1].access, Access::Write);
        // Majse's id is not Mark's to replace.
        assert!(!calendars.disconnect("mark", "m").await.unwrap());
    }
}

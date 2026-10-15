//! Google's sign-in and the Google Calendar API, for a Gmail or
//! Workspace account's primary calendar.

use crate::error::{Error, Result};
use crate::graph::{Tokens, first_line, http_error, instant};
use crate::model::{Access, Event, MailAccess, NewEvent, Services};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::{Value, json};

const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

/// The household's OAuth client in Google Cloud.
#[derive(Debug, Clone)]
pub struct GoogleApp {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// What an account asks Google to grant: exactly what its ticks need,
/// nothing else in the account.
pub fn scopes(services: Services) -> String {
    let mut scopes = Vec::new();
    match services.calendar {
        Some(Access::Read) => scopes.push("https://www.googleapis.com/auth/calendar.readonly"),
        Some(Access::Write) => scopes.push("https://www.googleapis.com/auth/calendar.events"),
        None => {}
    }
    match services.mail {
        Some(MailAccess::Read) => scopes.push("https://www.googleapis.com/auth/gmail.readonly"),
        Some(MailAccess::Send) => scopes.extend([
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/gmail.send",
        ]),
        None => {}
    }
    scopes.join(" ")
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl GoogleApp {
    /// `access_type=offline` and `prompt=consent` together are what make
    /// Google hand back a refresh token every time, not only the first.
    pub fn authorize_url(&self, state: &str, services: Services) -> String {
        format!(
            "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent%20select_account&state={}",
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&scopes(services)),
            urlencoding::encode(state),
        )
    }

    pub async fn exchange(&self, http: &reqwest::Client, code: &str) -> Result<Tokens> {
        self.token(
            http,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
            ],
            None,
        )
        .await
    }

    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        refresh_token: &str,
        name: &str,
    ) -> Result<Tokens> {
        self.token(
            http,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
            Some(name),
        )
        .await
    }

    async fn token(
        &self,
        http: &reqwest::Client,
        form: &[(&str, &str)],
        connection: Option<&str>,
    ) -> Result<Tokens> {
        let mut fields = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        fields.extend_from_slice(form);
        let response = http
            .post(TOKEN)
            .form(&fields)
            .send()
            .await
            .map_err(http_error)?;
        let status = response.status();
        let body = response.text().await.map_err(http_error)?;
        if !status.is_success() {
            let refused: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let error = refused.get("error").and_then(Value::as_str).unwrap_or("");
            if let Some(name) = connection
                && error == "invalid_grant"
            {
                return Err(Error::Reconnect {
                    name: name.to_string(),
                });
            }
            let said = refused
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or(if error.is_empty() { &body } else { error });
            return Err(Error::Provider {
                provider: "Google",
                status: status.as_u16(),
                reason: first_line(said),
            });
        }
        let tokens: TokenResponse = serde_json::from_str(&body).map_err(|e| Error::Provider {
            provider: "Google",
            status: status.as_u16(),
            reason: format!("an answer that would not read: {e}"),
        })?;
        Ok(Tokens {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: Utc::now() + chrono::Duration::seconds(tokens.expires_in.unwrap_or(3600)),
        })
    }
}

/// The account a token belongs to, asked of what the token may read:
/// a primary calendar is named after its account's address, and so is
/// a Gmail profile. Neither needs a scope beyond the service's own.
pub async fn account(
    http: &reqwest::Client,
    access_token: &str,
    services: Services,
) -> Result<String> {
    let (url, field) = if services.calendar.is_some() {
        (format!("{EVENTS}?maxResults=1"), "summary")
    } else {
        (GMAIL_PROFILE.to_string(), "emailAddress")
    };
    let body = send(http, access_token, reqwest::Method::GET, &url, None).await?;
    Ok(body
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Google account")
        .to_string())
}

const GMAIL_PROFILE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

/// The events overlapping `[from, to)`, repeating ones as their single
/// occurrences, in the house's time zone.
pub async fn events(
    http: &reqwest::Client,
    access_token: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz: Tz,
) -> Result<Vec<Event>> {
    let url = format!(
        "{EVENTS}?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=250",
        urlencoding::encode(&from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        urlencoding::encode(&to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    );
    let body = send(http, access_token, reqwest::Method::GET, &url, None).await?;
    Ok(body
        .get("items")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|e| read_event(e, tz)).collect())
        .unwrap_or_default())
}

pub async fn create(
    http: &reqwest::Client,
    access_token: &str,
    event: &NewEvent,
    tz: Tz,
) -> Result<Event> {
    let made = send(
        http,
        access_token,
        reqwest::Method::POST,
        EVENTS,
        Some(event_body(event, tz)),
    )
    .await?;
    read_event(&made, tz).ok_or_else(|| unreadable("the new event"))
}

pub async fn update(
    http: &reqwest::Client,
    access_token: &str,
    id: &str,
    event: &NewEvent,
    tz: Tz,
) -> Result<Event> {
    let changed = send(
        http,
        access_token,
        reqwest::Method::PATCH,
        &format!("{EVENTS}/{}", urlencoding::encode(id)),
        Some(event_body(event, tz)),
    )
    .await?;
    read_event(&changed, tz).ok_or_else(|| unreadable("the changed event"))
}

pub async fn delete(http: &reqwest::Client, access_token: &str, id: &str) -> Result<()> {
    send(
        http,
        access_token,
        reqwest::Method::DELETE,
        &format!("{EVENTS}/{}", urlencoding::encode(id)),
        None,
    )
    .await
    .map(|_| ())
}

/// A timed event as instants; an all-day one as dates, the last one
/// exclusive as Google counts them.
pub fn event_body(event: &NewEvent, tz: Tz) -> Value {
    let (start, end) = if event.all_day {
        let first = event.start.date();
        let last = event.end.date().max(first);
        let after = if last > first {
            last
        } else {
            first.succ_opt().unwrap_or(first)
        };
        (
            json!({ "date": first.format("%Y-%m-%d").to_string() }),
            json!({ "date": after.format("%Y-%m-%d").to_string() }),
        )
    } else {
        let at = |t| instant(t, tz).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        (
            json!({ "dateTime": at(event.start) }),
            json!({ "dateTime": at(event.end) }),
        )
    };
    let mut body = json!({ "summary": event.subject, "start": start, "end": end });
    if let Some(place) = &event.location {
        body["location"] = json!(place);
    }
    body
}

pub fn read_event(value: &Value, tz: Tz) -> Option<Event> {
    if value.get("status").and_then(Value::as_str) == Some("cancelled") {
        return None;
    }
    let id = value.get("id")?.as_str()?.to_string();
    let start = value.get("start")?;
    let all_day = start.get("date").is_some();
    let when = |key: &str| -> Option<chrono::NaiveDateTime> {
        let point = value.get(key)?;
        if let Some(date) = point.get("date").and_then(Value::as_str) {
            return Some(
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .ok()?
                    .and_time(NaiveTime::MIN),
            );
        }
        let stamp = point.get("dateTime")?.as_str()?;
        Some(
            DateTime::parse_from_rfc3339(stamp)
                .ok()?
                .with_timezone(&tz)
                .naive_local(),
        )
    };
    Some(Event {
        id,
        subject: value
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("(no title)")
            .to_string(),
        start: when("start")?,
        end: when("end")?,
        all_day,
        location: value
            .get("location")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        recurring: value.get("recurringEventId").is_some(),
    })
}

fn unreadable(what: &str) -> Error {
    Error::Provider {
        provider: "Google",
        status: 200,
        reason: format!("{what} came back unreadable"),
    }
}

pub(crate) async fn send(
    http: &reqwest::Client,
    access_token: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = http.request(method, url).bearer_auth(access_token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(http_error)?;
    let status = response.status();
    let text = response.text().await.map_err(http_error)?;
    if !status.is_success() {
        let reason = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .unwrap_or(text);
        return Err(Error::Provider {
            provider: "Google",
            status: status.as_u16(),
            reason: first_line(&reason),
        });
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| Error::Provider {
        provider: "Google",
        status: status.as_u16(),
        reason: format!("an answer that would not read: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn copenhagen() -> Tz {
        "Europe/Copenhagen".parse().unwrap()
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn asks_google_for_only_the_access_chosen() {
        let app = GoogleApp {
            client_id: "id".into(),
            client_secret: "s".into(),
            redirect_uri: "https://niles.example/calendars/google/callback".into(),
        };
        let read = app.authorize_url(
            "st",
            Services {
                calendar: Some(Access::Read),
                mail: None,
            },
        );
        assert!(read.contains("calendar.readonly"));
        assert!(!read.contains("calendar.events"));
        assert!(read.contains("access_type=offline"));
        assert!(
            app.authorize_url(
                "st",
                Services {
                    calendar: Some(Access::Write),
                    mail: None
                }
            )
            .contains("calendar.events")
        );
        assert!(!read.contains("gmail"), "{read}");
    }

    #[test]
    fn asks_for_gmail_only_when_mail_is_ticked() {
        let read = scopes(Services {
            calendar: None,
            mail: Some(MailAccess::Read),
        });
        assert_eq!(read, "https://www.googleapis.com/auth/gmail.readonly");
        let send = scopes(Services {
            calendar: Some(Access::Write),
            mail: Some(MailAccess::Send),
        });
        assert!(send.ends_with("gmail.readonly https://www.googleapis.com/auth/gmail.send"));
        assert!(send.starts_with("https://www.googleapis.com/auth/calendar.events "));
    }

    #[test]
    fn reads_timed_all_day_and_repeating_events() {
        let timed = read_event(
            &json!({
                "id": "g1", "summary": "Yoga", "location": "Studio",
                "start": { "dateTime": "2026-10-12T07:00:00Z" },
                "end": { "dateTime": "2026-10-12T08:00:00Z" },
                "recurringEventId": "series",
            }),
            copenhagen(),
        )
        .unwrap();
        assert_eq!(timed.start, at("2026-10-12 09:00"));
        assert!(timed.recurring);
        let day = read_event(
            &json!({
                "id": "g2", "summary": "Holiday",
                "start": { "date": "2026-10-20" }, "end": { "date": "2026-10-21" },
            }),
            copenhagen(),
        )
        .unwrap();
        assert!(day.all_day);
        assert_eq!(day.start, at("2026-10-20 00:00"));
        assert!(
            read_event(
                &json!({ "id": "g3", "status": "cancelled", "start": { "date": "2026-10-20" } }),
                copenhagen()
            )
            .is_none()
        );
    }

    #[test]
    fn writes_times_as_instants_and_days_as_dates() {
        let body = event_body(
            &NewEvent {
                subject: "Dentist".into(),
                start: at("2026-10-15 10:00"),
                end: at("2026-10-15 11:00"),
                all_day: false,
                location: None,
            },
            copenhagen(),
        );
        assert_eq!(body["start"]["dateTime"], "2026-10-15T08:00:00Z");
        let day = event_body(
            &NewEvent {
                subject: "Trip".into(),
                start: at("2026-10-20 00:00"),
                end: at("2026-10-20 00:00"),
                all_day: true,
                location: None,
            },
            copenhagen(),
        );
        assert_eq!(day["start"]["date"], "2026-10-20");
        assert_eq!(day["end"]["date"], "2026-10-21");
    }
}

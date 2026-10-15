//! Microsoft identity and Graph, as far as a calendar needs them: the
//! sign-in a person goes through, the tokens it yields, and the events.
//!
//! One app registration serves both kinds of account — Outlook.com and
//! Entra ID (work or school) — through the `common` authority.
//!
//! Times go to and come from Graph in UTC and are turned into the
//! house's time zone here: Graph's own zone names are Windows names,
//! and chrono knows the IANA ones.

use crate::error::{Error, Result};
use crate::model::{Access, Event, MailAccess, NewEvent, Services};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::{Value, json};

const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
pub(crate) const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// The household's app registration, as one request needs it. Built
/// from the config on every use, so new keys take hold without a
/// restart.
#[derive(Debug, Clone)]
pub struct MicrosoftApp {
    pub client_id: String,
    pub client_secret: String,
    /// Compared by Microsoft letter for letter, at sign-in and again at
    /// the token exchange.
    pub redirect_uri: String,
}

/// What a sign-in or a refresh hands back.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    /// Microsoft rotates it on every refresh, and the one that came
    /// back is the one to keep; Google sends one only at sign-in.
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// What an account asks Microsoft to grant: exactly what its ticks
/// need. `offline_access` is what yields a refresh token; `User.Read`
/// names the account.
pub fn scopes(services: Services) -> String {
    let mut scopes = vec!["offline_access", "User.Read"];
    match services.calendar {
        Some(Access::Read) => scopes.push("Calendars.Read"),
        Some(Access::Write) => scopes.push("Calendars.ReadWrite"),
        None => {}
    }
    match services.mail {
        Some(MailAccess::Read) => scopes.push("Mail.Read"),
        Some(MailAccess::Send) => scopes.extend(["Mail.Read", "Mail.Send"]),
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

#[derive(Deserialize)]
struct OAuthError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

impl MicrosoftApp {
    /// Where to send the person's browser. `select_account`, because a
    /// person with two accounts is exactly who this is for, and the
    /// browser would otherwise sign the second in as the first.
    pub fn authorize_url(&self, state: &str, services: Services) -> String {
        format!(
            "{AUTHORITY}/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query&scope={}&state={}&prompt=select_account",
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&scopes(services)),
            urlencoding::encode(state),
        )
    }

    /// Turn the code the callback brought into tokens.
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        code: &str,
        services: Services,
    ) -> Result<Tokens> {
        self.token(
            http,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("scope", &scopes(services)),
            ],
            None,
        )
        .await
    }

    /// A fresh access token from a stored refresh token. `name` is the
    /// connection's, for the error a revoked sign-in turns into.
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        refresh_token: &str,
        services: Services,
        name: &str,
    ) -> Result<Tokens> {
        self.token(
            http,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", &scopes(services)),
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
            .post(format!("{AUTHORITY}/token"))
            .form(&fields)
            .send()
            .await
            .map_err(http_error)?;
        let status = response.status();
        let body = response.text().await.map_err(http_error)?;
        if !status.is_success() {
            let refused: OAuthError = serde_json::from_str(&body).unwrap_or(OAuthError {
                error: String::new(),
                error_description: body.clone(),
            });
            // A revoked or expired sign-in: nothing but connecting again
            // will fix it, and that is what the person should be told.
            if let Some(name) = connection
                && refused.error == "invalid_grant"
            {
                return Err(Error::Reconnect {
                    name: name.to_string(),
                });
            }
            return Err(Error::Microsoft {
                status: status.as_u16(),
                reason: first_line(if refused.error_description.is_empty() {
                    &refused.error
                } else {
                    &refused.error_description
                }),
            });
        }
        let tokens: TokenResponse = serde_json::from_str(&body).map_err(|e| Error::Microsoft {
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

/// The address of the account a token belongs to: what the profile
/// shows beside the connection's name.
pub async fn account(http: &reqwest::Client, access_token: &str) -> Result<String> {
    let me = get(
        http,
        access_token,
        &format!("{GRAPH}/me?$select=mail,userPrincipalName,displayName"),
    )
    .await?;
    Ok(["mail", "userPrincipalName", "displayName"]
        .iter()
        .find_map(|k| me.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
        .unwrap_or("Microsoft account")
        .to_string())
}

/// The events overlapping `[from, to)`, recurring ones as their single
/// occurrences, in the house's time zone.
pub async fn events(
    http: &reqwest::Client,
    access_token: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz: Tz,
) -> Result<Vec<Event>> {
    let url = format!(
        "{GRAPH}/me/calendarView?startDateTime={}&endDateTime={}&$select=id,subject,start,end,isAllDay,location,isCancelled,showAs,type&$orderby=start/dateTime&$top=100",
        urlencoding::encode(&from.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        urlencoding::encode(&to.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
    );
    let body = get(http, access_token, &url).await?;
    let items = body
        .get("value")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .filter(|e| {
            !e.get("isCancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(|e| read_event(e, tz))
        .collect())
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
        &format!("{GRAPH}/me/events"),
        Some(event_body(event, tz)),
    )
    .await?;
    read_event(&made, tz).ok_or_else(|| Error::Microsoft {
        status: 200,
        reason: "the new event came back unreadable".into(),
    })
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
        &format!("{GRAPH}/me/events/{}", urlencoding::encode(id)),
        Some(event_body(event, tz)),
    )
    .await?;
    read_event(&changed, tz).ok_or_else(|| Error::Microsoft {
        status: 200,
        reason: "the changed event came back unreadable".into(),
    })
}

pub async fn delete(http: &reqwest::Client, access_token: &str, id: &str) -> Result<()> {
    send(
        http,
        access_token,
        reqwest::Method::DELETE,
        &format!("{GRAPH}/me/events/{}", urlencoding::encode(id)),
        None,
    )
    .await
    .map(|_| ())
}

/// The body Graph takes for a new or changed event. A timed event goes
/// in UTC, which is exact. An all-day one is a date, not an instant:
/// Graph wants it at midnight in a named zone, and UTC is the one name
/// it is sure to know — the same zone it reports all-day events in.
pub fn event_body(event: &NewEvent, tz: Tz) -> Value {
    let (start, end) = if event.all_day {
        let first = event.start.date();
        let last = event.end.date().max(first);
        // A one-day event ends at the next midnight.
        let after = if last > first {
            last
        } else {
            first.succ_opt().unwrap_or(first)
        };
        (midnight(first), midnight(after))
    } else {
        (utc(event.start, tz), utc(event.end, tz))
    };
    let mut body = json!({
        "subject": event.subject,
        "isAllDay": event.all_day,
        "start": { "dateTime": start, "timeZone": "UTC" },
        "end": { "dateTime": end, "timeZone": "UTC" },
    });
    if let Some(place) = &event.location {
        body["location"] = json!({ "displayName": place });
    }
    body
}

fn midnight(day: NaiveDate) -> String {
    format!("{}T00:00:00", day.format("%Y-%m-%d"))
}

fn utc(local: NaiveDateTime, tz: Tz) -> String {
    instant(local, tz).format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// A local wall-clock time in the house as an instant. A time that does
/// not exist (the hour the clocks skip) is taken as the moment after.
pub(crate) fn instant(local: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(local + chrono::Duration::hours(1)))
                .earliest()
        })
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&local))
}

/// One event from Graph, in the house's time. All-day events are dates
/// and are not moved between zones: one on the 12th is on the 12th.
pub fn read_event(value: &Value, tz: Tz) -> Option<Event> {
    let id = value.get("id")?.as_str()?.to_string();
    let all_day = value
        .get("isAllDay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let when = |key: &str| -> Option<NaiveDateTime> {
        let stamp = value.get(key)?.get("dateTime")?.as_str()?;
        let naive = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
        Some(if all_day {
            naive
        } else {
            Utc.from_utc_datetime(&naive)
                .with_timezone(&tz)
                .naive_local()
        })
    };
    let location = value
        .get("location")
        .and_then(|l| l.get("displayName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    Some(Event {
        id,
        subject: value
            .get("subject")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("(no title)")
            .to_string(),
        start: when("start")?,
        end: when("end")?,
        all_day,
        location,
        recurring: matches!(
            value.get("type").and_then(Value::as_str),
            Some("occurrence" | "exception")
        ),
    })
}

pub(crate) async fn get(http: &reqwest::Client, access_token: &str, url: &str) -> Result<Value> {
    send(http, access_token, reqwest::Method::GET, url, None).await
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
        return Err(Error::Microsoft {
            status: status.as_u16(),
            reason: first_line(&reason),
        });
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| Error::Microsoft {
        status: status.as_u16(),
        reason: format!("an answer that would not read: {e}"),
    })
}

pub(crate) fn http_error(e: reqwest::Error) -> Error {
    Error::Http {
        reason: e.to_string(),
    }
}

/// Microsoft's refusals run to several lines of trace ids.
pub(crate) fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copenhagen() -> Tz {
        "Europe/Copenhagen".parse().unwrap()
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn asks_only_for_the_access_chosen() {
        let app = MicrosoftApp {
            client_id: "id".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://niles.example/calendars/microsoft/callback".into(),
        };
        let read = app.authorize_url(
            "s1",
            Services {
                calendar: Some(Access::Read),
                mail: None,
            },
        );
        assert!(read.contains("Calendars.Read&"), "{read}");
        assert!(!read.contains("ReadWrite"), "{read}");
        assert!(read.contains("offline_access"));
        assert!(read.contains(
            "redirect_uri=https%3A%2F%2Fniles.example%2Fcalendars%2Fmicrosoft%2Fcallback"
        ));
        assert!(
            app.authorize_url(
                "s1",
                Services {
                    calendar: Some(Access::Write),
                    mail: None
                }
            )
            .contains("Calendars.ReadWrite")
        );
        assert!(!read.contains("Mail."), "{read}");
    }

    #[test]
    fn asks_for_mail_only_when_it_is_ticked() {
        let both = scopes(Services {
            calendar: Some(Access::Read),
            mail: Some(MailAccess::Read),
        });
        assert_eq!(both, "offline_access User.Read Calendars.Read Mail.Read");
        let send = scopes(Services {
            calendar: None,
            mail: Some(MailAccess::Send),
        });
        assert_eq!(send, "offline_access User.Read Mail.Read Mail.Send");
    }

    #[test]
    fn reads_a_timed_event_into_the_house_time() {
        let event = read_event(
            &json!({
                "id": "AAMk1",
                "subject": "Standup",
                "isAllDay": false,
                "start": { "dateTime": "2026-10-12T07:00:00.0000000", "timeZone": "UTC" },
                "end": { "dateTime": "2026-10-12T07:15:00.0000000", "timeZone": "UTC" },
                "location": { "displayName": "Teams" },
            }),
            copenhagen(),
        )
        .unwrap();
        assert_eq!(event.start, at("2026-10-12 09:00"));
        assert_eq!(event.end, at("2026-10-12 09:15"));
        assert_eq!(event.location.as_deref(), Some("Teams"));
    }

    #[test]
    fn keeps_an_all_day_event_on_its_own_date() {
        let event = read_event(
            &json!({
                "id": "AAMk2",
                "subject": "",
                "isAllDay": true,
                "start": { "dateTime": "2026-10-12T00:00:00.0000000", "timeZone": "UTC" },
                "end": { "dateTime": "2026-10-13T00:00:00.0000000", "timeZone": "UTC" },
                "location": { "displayName": "" },
            }),
            copenhagen(),
        )
        .unwrap();
        assert_eq!(event.start, at("2026-10-12 00:00"));
        assert_eq!(event.subject, "(no title)");
        assert_eq!(event.location, None);
    }

    #[test]
    fn writes_a_timed_event_in_utc() {
        let body = event_body(
            &NewEvent {
                subject: "Dentist".into(),
                start: at("2026-10-15 10:00"),
                end: at("2026-10-15 11:00"),
                all_day: false,
                location: Some("Aarhus".into()),
            },
            copenhagen(),
        );
        assert_eq!(body["start"]["dateTime"], "2026-10-15T08:00:00");
        assert_eq!(body["end"]["dateTime"], "2026-10-15T09:00:00");
        assert_eq!(body["start"]["timeZone"], "UTC");
        assert_eq!(body["location"]["displayName"], "Aarhus");
    }

    #[test]
    fn writes_a_one_day_event_to_the_next_midnight() {
        let body = event_body(
            &NewEvent {
                subject: "Majse's birthday".into(),
                start: at("2026-11-02 00:00"),
                end: at("2026-11-02 00:00"),
                all_day: true,
                location: None,
            },
            copenhagen(),
        );
        assert_eq!(body["start"]["dateTime"], "2026-11-02T00:00:00");
        assert_eq!(body["end"]["dateTime"], "2026-11-03T00:00:00");
        assert_eq!(body["isAllDay"], true);
    }
}

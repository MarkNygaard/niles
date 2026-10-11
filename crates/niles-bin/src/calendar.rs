//! The calendar tools: the speaker's own calendars, read and — where a
//! connection allows it — written.
//!
//! Whose calendars comes from [`profile::SPEAKER`], set for the turn by
//! the dispatcher, never from an argument: the model cannot ask for
//! somebody else's by name, and an unrecognised voice has none.

use crate::profile;
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use niles_calendar::{Calendars, Change, Listed, MicrosoftApp, NewEvent};
use niles_config::ConfigStore;
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::sync::Arc;

struct Shared {
    calendars: Arc<Calendars>,
    config: Arc<ConfigStore>,
    tz: chrono_tz::Tz,
}

impl Shared {
    fn person(&self, tool: &str) -> Result<String> {
        profile::SPEAKER
            .try_with(Clone::clone)
            .ok()
            .flatten()
            .ok_or_else(|| {
                invalid(
                    tool,
                    "I don't know whose voice this is, so I can't tell whose calendar to look at"
                        .into(),
                )
            })
    }

    /// The app registration, read per call so keys typed into the app
    /// take hold without a restart. A refresh needs no redirect.
    fn app(&self, tool: &str) -> Result<MicrosoftApp> {
        let cfg = self.config.current();
        let microsoft = cfg
            .integrations
            .microsoft
            .as_ref()
            .filter(|m| m.enabled)
            .ok_or_else(|| invalid(tool, "Microsoft calendars are switched off".into()))?;
        let (client_id, client_secret) = microsoft
            .resolve_credentials()
            .map_err(|e| invalid(tool, e.to_string()))?;
        Ok(MicrosoftApp {
            client_id,
            client_secret,
            redirect_uri: String::new(),
        })
    }

    fn listed(&self, listed: &Listed) -> Value {
        let e = &listed.event;
        let mut out = json!({
            "ref": listed.reference,
            "calendar": listed.calendar,
            "title": e.subject,
            "all_day": e.all_day,
        });
        if e.all_day {
            out["date"] = json!(e.start.format("%A %Y-%m-%d").to_string());
            let last = e.end.date().pred_opt().unwrap_or(e.end.date());
            if last > e.start.date() {
                out["until"] = json!(last.format("%A %Y-%m-%d").to_string());
            }
        } else {
            out["start"] = json!(e.start.format("%A %Y-%m-%d %H:%M").to_string());
            out["end"] = json!(e.end.format("%Y-%m-%d %H:%M").to_string());
        }
        if let Some(place) = &e.location {
            out["location"] = json!(place);
        }
        out
    }
}

fn invalid(tool: &str, reason: String) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason,
    }
}

/// "2026-10-15", "2026-10-15T10:00", "2026-10-15 10:00" (seconds
/// allowed). A date alone is its midnight.
pub fn parse_when(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(at) = NaiveDateTime::parse_from_str(value, format) {
            return Some(at);
        }
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_time(NaiveTime::MIN))
}

fn when(tool: &str, args: &Value, key: &str) -> Result<Option<NaiveDateTime>> {
    match args.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(raw) => parse_when(raw).map(Some).ok_or_else(|| {
            invalid(
                tool,
                format!("{key} {raw:?} is not a date (YYYY-MM-DD) or a time (YYYY-MM-DDTHH:MM)"),
            )
        }),
    }
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn failed(tool: &str, e: niles_calendar::Error) -> Error {
    invalid(tool, e.to_string())
}

pub struct CalendarEvents(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for CalendarEvents {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "calendar_events".into(),
            description: "The calendar of the person speaking — all their connected calendars \
                together: 'what's on my calendar tomorrow', 'when is my next meeting', 'am I \
                free on Friday afternoon'. Only theirs: nobody can ask for someone else's. \
                Times are local. Each event has a ref (e1, e2…) to change or delete it by. Read \
                titles naturally and briefly; say which calendar only when it helps."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Start of the window, YYYY-MM-DD or YYYY-MM-DDTHH:MM, local. Default: now." },
                    "to": { "type": "string", "description": "End of the window, same form. Default: a day after from." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "calendar_events";
        let s = &self.0;
        let person = s.person(TOOL)?;
        let now = Utc::now().with_timezone(&s.tz).naive_local();
        let from = when(TOOL, &args, "from")?.unwrap_or(now);
        let to = when(TOOL, &args, "to")?
            .filter(|to| *to > from)
            .unwrap_or(from + Duration::days(1));
        let connections = s
            .calendars
            .connections(&person)
            .await
            .map_err(|e| failed(TOOL, e))?;
        if connections.is_empty() {
            return Ok(json!({
                "events": [],
                "note": "This person has not connected a calendar. They do that in their profile in the Niles app."
            }));
        }
        let app = s.app(TOOL)?;
        let utc = |at: NaiveDateTime| {
            s.tz.from_local_datetime(&at)
                .earliest()
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|| Utc.from_utc_datetime(&at))
        };
        let (listed, problems) = s
            .calendars
            .events(&app, &person, utc(from), utc(to), s.tz)
            .await
            .map_err(|e| failed(TOOL, e))?;
        Ok(json!({
            "from": from.format("%A %Y-%m-%d %H:%M").to_string(),
            "to": to.format("%A %Y-%m-%d %H:%M").to_string(),
            "events": listed.iter().map(|l| s.listed(l)).collect::<Vec<_>>(),
            "calendars": connections.iter().map(|c| json!({ "name": c.name, "access": c.access.as_str() })).collect::<Vec<_>>(),
            "problems": problems,
        }))
    }
}

pub struct CreateCalendarEvent(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for CreateCalendarEvent {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "create_calendar_event".into(),
            description: "Put an event in the speaker's calendar: 'add the dentist on Thursday \
                at ten'. First read it back and ask — 'Dentist, Thursday 15 October at 10:00, \
                in your Private calendar. Shall I?' — and call this only after a yes. Only a \
                calendar connected with write access takes it; if they have several, ask which \
                unless they said. Use current_datetime to turn 'Thursday' into a date."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["title", "start"],
                "properties": {
                    "title": { "type": "string" },
                    "start": { "type": "string", "description": "YYYY-MM-DDTHH:MM local, or YYYY-MM-DD for an all-day event." },
                    "end": { "type": "string", "description": "Same form. Default: an hour after start, or the same day for an all-day event." },
                    "all_day": { "type": "boolean" },
                    "location": { "type": "string" },
                    "calendar": { "type": "string", "description": "The calendar's name, e.g. Private, when they have more than one they can write to." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "create_calendar_event";
        let s = &self.0;
        let person = s.person(TOOL)?;
        let title = text(&args, "title").ok_or_else(|| invalid(TOOL, "no title".into()))?;
        let start = when(TOOL, &args, "start")?.ok_or_else(|| invalid(TOOL, "no start".into()))?;
        let all_day = args
            .get("all_day")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                args.get("start")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.trim().len() == 10)
            });
        let end = when(TOOL, &args, "end")?
            .filter(|end| *end >= start)
            .unwrap_or(if all_day {
                start
            } else {
                start + Duration::hours(1)
            });
        let event = NewEvent {
            subject: title,
            start,
            end,
            all_day,
            location: text(&args, "location"),
        };
        let app = s.app(TOOL)?;
        let made = s
            .calendars
            .create(
                &app,
                &person,
                text(&args, "calendar").as_deref(),
                &event,
                s.tz,
            )
            .await
            .map_err(|e| failed(TOOL, e))?;
        tracing::info!(
            "[calendars] {person}: made {:?} in {}",
            made.event.subject,
            made.calendar
        );
        Ok(json!({ "created": s.listed(&made) }))
    }
}

pub struct UpdateCalendarEvent(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for UpdateCalendarEvent {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "update_calendar_event".into(),
            description: "Change one of the speaker's events, by the ref calendar_events gave \
                it: a new time, title or place. Give only what changes; a new start alone keeps \
                its length. Read the change back and ask first, and call this only after a yes. \
                Only on a calendar connected with write access."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["ref"],
                "properties": {
                    "ref": { "type": "string" },
                    "title": { "type": "string" },
                    "start": { "type": "string" },
                    "end": { "type": "string" },
                    "location": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "update_calendar_event";
        let s = &self.0;
        let person = s.person(TOOL)?;
        let reference = text(&args, "ref").ok_or_else(|| invalid(TOOL, "no ref".into()))?;
        let change = Change {
            subject: text(&args, "title"),
            start: when(TOOL, &args, "start")?,
            end: when(TOOL, &args, "end")?,
            all_day: None,
            location: text(&args, "location"),
        };
        let app = s.app(TOOL)?;
        let changed = s
            .calendars
            .update(&app, &person, &reference, &change, s.tz)
            .await
            .map_err(|e| failed(TOOL, e))?;
        tracing::info!("[calendars] {person}: changed {:?}", changed.event.subject);
        Ok(json!({ "updated": s.listed(&changed) }))
    }
}

pub struct DeleteCalendarEvent(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for DeleteCalendarEvent {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "delete_calendar_event".into(),
            description: "Delete one of the speaker's events, by the ref calendar_events gave \
                it. Say which event and ask first — 'Delete the dentist on Thursday?' — and call \
                this only after a yes. Only on a calendar connected with write access."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["ref"],
                "properties": { "ref": { "type": "string" } },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "delete_calendar_event";
        let s = &self.0;
        let person = s.person(TOOL)?;
        let reference = text(&args, "ref").ok_or_else(|| invalid(TOOL, "no ref".into()))?;
        let app = s.app(TOOL)?;
        let gone = s
            .calendars
            .delete(&app, &person, &reference)
            .await
            .map_err(|e| failed(TOOL, e))?;
        tracing::info!("[calendars] {person}: deleted {:?}", gone.event.subject);
        Ok(json!({ "deleted": s.listed(&gone) }))
    }
}

pub fn register(
    reg: &mut ToolRegistry,
    calendars: Arc<Calendars>,
    config: Arc<ConfigStore>,
    tz: chrono_tz::Tz,
) {
    let shared = Arc::new(Shared {
        calendars,
        config,
        tz,
    });
    reg.register(Box::new(CalendarEvents(shared.clone())));
    reg.register(Box::new(CreateCalendarEvent(shared.clone())));
    reg.register(Box::new(UpdateCalendarEvent(shared.clone())));
    reg.register(Box::new(DeleteCalendarEvent(shared)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_calendar::MemoryConnections;

    fn tools() -> Arc<Shared> {
        let store = Arc::new(MemoryConnections::new());
        let graph = Arc::new(niles_calendar::Graph::new(reqwest::Client::new()));
        Arc::new(Shared {
            calendars: Arc::new(Calendars::new(store, graph)),
            config: Arc::new(ConfigStore::from_str_in_memory("").unwrap()),
            tz: "Europe/Copenhagen".parse().unwrap(),
        })
    }

    #[test]
    fn reads_the_dates_a_model_writes() {
        assert_eq!(
            parse_when("2026-10-15T10:00"),
            NaiveDate::from_ymd_opt(2026, 10, 15)
                .unwrap()
                .and_hms_opt(10, 0, 0)
        );
        assert_eq!(
            parse_when("2026-10-15"),
            NaiveDate::from_ymd_opt(2026, 10, 15)
                .unwrap()
                .and_hms_opt(0, 0, 0)
        );
        assert_eq!(parse_when("Thursday"), None);
    }

    #[tokio::test]
    async fn an_unknown_voice_has_no_calendar() {
        let events = CalendarEvents(tools());
        let unknown = profile::SPEAKER
            .scope(None, events.execute(json!({})))
            .await;
        assert!(unknown.unwrap_err().to_string().contains("whose voice"));
    }

    #[tokio::test]
    async fn a_person_without_a_calendar_is_told_where_to_connect_one() {
        let events = CalendarEvents(tools());
        let out = profile::SPEAKER
            .scope(Some("mark".into()), events.execute(json!({})))
            .await
            .unwrap();
        assert!(out["note"].as_str().unwrap().contains("profile"));
    }
}

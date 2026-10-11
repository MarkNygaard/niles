//! Just enough iCalendar (RFC 5545) for CalDAV: reading the events a
//! server sends, and writing the ones Niles makes.
//!
//! Not a general parser. It reads VEVENTs — their times in UTC, in a
//! named zone, or floating; all-day dates; a duration in place of an
//! end — and leaves everything else in an event as it found it.

use crate::model::{Event, NewEvent};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// One content line: its name, its parameters, its value.
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Line {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Lines folded across several (a line starting with a space or a tab
/// continues the one before), joined.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    lines
}

fn parse_line(raw: &str) -> Option<Line> {
    // The value starts at the first colon outside a quoted parameter.
    let mut quoted = false;
    let split = raw.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    let (head, value) = (&raw[..split.0], &raw[split.0 + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect();
    Some(Line {
        name,
        params,
        value: value.to_string(),
    })
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// A DTSTART or DTEND in the house's time, and whether it is a date.
fn when(line: &Line, tz: Tz) -> Option<(NaiveDateTime, bool)> {
    let value = line.value.trim();
    if line.param("VALUE") == Some("DATE") || value.len() == 8 {
        let day = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((day.and_time(NaiveTime::MIN), true));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((
            Utc.from_utc_datetime(&naive)
                .with_timezone(&tz)
                .naive_local(),
            false,
        ));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    // In a named zone, moved to the house's; with none, it is floating
    // and means the same wall clock wherever it is read.
    let zone: Option<Tz> = line.param("TZID").and_then(|z| z.parse().ok());
    Some(match zone {
        Some(zone) => (
            zone.from_local_datetime(&naive)
                .earliest()
                .map(|t| t.with_timezone(&tz).naive_local())
                .unwrap_or(naive),
            false,
        ),
        None => (naive, false),
    })
}

/// "PT1H30M", "P1D", "P2W" — the durations events are written with.
fn duration(value: &str) -> Option<chrono::Duration> {
    let value = value.trim().trim_start_matches('+');
    let rest = value.strip_prefix('P')?;
    let (days_part, time_part) = rest.split_once('T').unwrap_or((rest, ""));
    let mut total = chrono::Duration::zero();
    let mut number = String::new();
    for c in days_part.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: i64 = number.parse().ok()?;
        number.clear();
        total += match c {
            'W' => chrono::Duration::weeks(n),
            'D' => chrono::Duration::days(n),
            _ => return None,
        };
    }
    for c in time_part.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: i64 = number.parse().ok()?;
        number.clear();
        total += match c {
            'H' => chrono::Duration::hours(n),
            'M' => chrono::Duration::minutes(n),
            'S' => chrono::Duration::seconds(n),
            _ => return None,
        };
    }
    Some(total)
}

/// Every event in a calendar resource, in the house's time. `href` is
/// the resource's address, which is what identifies it to the server;
/// one occurrence of a repeating event adds its RECURRENCE-ID.
pub fn events(text: &str, href: &str, tz: Tz) -> Vec<Event> {
    let mut out = Vec::new();
    let mut current: Option<Vec<Line>> = None;
    let mut nested = 0usize;
    let mut repeats = false;
    for raw in unfold(text) {
        let Some(line) = parse_line(&raw) else {
            continue;
        };
        match (
            line.name.as_str(),
            line.value.trim().to_ascii_uppercase().as_str(),
        ) {
            ("BEGIN", "VEVENT") => current = Some(Vec::new()),
            ("END", "VEVENT") => {
                if let Some(lines) = current.take()
                    && let Some(event) = event_of(&lines, href, tz)
                {
                    out.push(event);
                }
            }
            ("BEGIN", _) if current.is_some() => nested += 1,
            ("END", _) if current.is_some() && nested > 0 => nested -= 1,
            _ => {
                if line.name == "RRULE" {
                    repeats = true;
                }
                if nested == 0
                    && let Some(lines) = current.as_mut()
                {
                    lines.push(line);
                }
            }
        }
    }
    // A series sent unexpanded is still a series: each of its events
    // repeats, whichever of them carried the rule.
    if repeats {
        for event in &mut out {
            event.recurring = true;
        }
    }
    out
}

fn event_of(lines: &[Line], href: &str, tz: Tz) -> Option<Event> {
    let get = |name: &str| lines.iter().find(|l| l.name == name);
    if get("STATUS").is_some_and(|s| s.value.eq_ignore_ascii_case("CANCELLED")) {
        return None;
    }
    let (start, all_day) = when(get("DTSTART")?, tz)?;
    let end = match (get("DTEND"), get("DURATION")) {
        (Some(end), _) => when(end, tz)?.0,
        (None, Some(length)) => start + duration(&length.value)?,
        (None, None) if all_day => start + chrono::Duration::days(1),
        (None, None) => start,
    };
    let occurrence = get("RECURRENCE-ID");
    let id = match occurrence {
        Some(at) => format!("{href}#{}", at.value.trim()),
        None => href.to_string(),
    };
    Some(Event {
        id,
        subject: get("SUMMARY")
            .map(|s| unescape(&s.value))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "(no title)".into()),
        start,
        end,
        all_day,
        location: get("LOCATION")
            .map(|l| unescape(&l.value).trim().to_string())
            .filter(|s| !s.is_empty()),
        recurring: occurrence.is_some() || get("RRULE").is_some(),
    })
}

/// Whether a resource is a repeating series.
pub fn repeats(text: &str) -> bool {
    unfold(text)
        .iter()
        .filter_map(|l| parse_line(l))
        .any(|l| l.name == "RRULE" || l.name == "RECURRENCE-ID")
}

fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y%m%dT%H%M%SZ").to_string()
}

/// The time lines of an event: instants in UTC, or dates.
fn times(event: &NewEvent, tz: Tz) -> [String; 2] {
    if event.all_day {
        let first = event.start.date();
        let last = event.end.date().max(first);
        let after = if last > first {
            last
        } else {
            first.succ_opt().unwrap_or(first)
        };
        [
            format!("DTSTART;VALUE=DATE:{}", first.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", after.format("%Y%m%d")),
        ]
    } else {
        [
            format!("DTSTART:{}", stamp(crate::graph::instant(event.start, tz))),
            format!("DTEND:{}", stamp(crate::graph::instant(event.end, tz))),
        ]
    }
}

/// A new calendar resource holding one event.
pub fn build(uid: &str, event: &NewEvent, tz: Tz) -> String {
    let [start, end] = times(event, tz);
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Niles//Calendar//EN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", stamp(Utc::now())),
        start,
        end,
        format!("SUMMARY:{}", escape(&event.subject)),
    ];
    if let Some(place) = &event.location {
        lines.push(format!("LOCATION:{}", escape(place)));
    }
    lines.extend(["END:VEVENT".to_string(), "END:VCALENDAR".into()]);
    lines.join("\r\n") + "\r\n"
}

/// An existing resource with its event's time, title and place
/// replaced — and everything else in it (alarms, notes, attendees)
/// kept as it was.
pub fn rewrite(text: &str, event: &NewEvent, tz: Tz) -> String {
    const REPLACED: &[&str] = &[
        "DTSTART",
        "DTEND",
        "DURATION",
        "SUMMARY",
        "LOCATION",
        "DTSTAMP",
        "LAST-MODIFIED",
    ];
    let [start, end] = times(event, tz);
    let mut out = Vec::new();
    let mut in_event = false;
    let mut nested = 0usize;
    for raw in unfold(text) {
        let line = parse_line(&raw);
        let name = line.as_ref().map(|l| l.name.as_str()).unwrap_or("");
        let value = line
            .as_ref()
            .map(|l| l.value.trim().to_ascii_uppercase())
            .unwrap_or_default();
        match (name, value.as_str()) {
            ("BEGIN", "VEVENT") => in_event = true,
            ("BEGIN", _) if in_event => nested += 1,
            ("END", "VEVENT") => {
                out.push(start.clone());
                out.push(end.clone());
                out.push(format!("SUMMARY:{}", escape(&event.subject)));
                if let Some(place) = &event.location {
                    out.push(format!("LOCATION:{}", escape(place)));
                }
                let now = stamp(Utc::now());
                out.push(format!("DTSTAMP:{now}"));
                out.push(format!("LAST-MODIFIED:{now}"));
                in_event = false;
            }
            ("END", _) if in_event && nested > 0 => nested -= 1,
            _ if in_event && nested == 0 && REPLACED.contains(&name) => continue,
            _ => {}
        }
        out.push(raw);
    }
    out.join("\r\n") + "\r\n"
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

    const DENTIST: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Copenhagen\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:abc\r\nDTSTART;TZID=Europe/Copenhagen:20261015T100000\r\nDTEND;TZID=Europe/Copenhagen:20261015T110000\r\nSUMMARY:Dentist\\, Dr. Hansen\r\nLOCATION:Aarhus \r\n C\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn reads_an_event_in_a_named_zone() {
        let read = events(DENTIST, "https://p1/cal/abc.ics", copenhagen());
        assert_eq!(read.len(), 1);
        let e = &read[0];
        assert_eq!(e.subject, "Dentist, Dr. Hansen");
        assert_eq!(e.location.as_deref(), Some("Aarhus C"));
        assert_eq!(e.start, at("2026-10-15 10:00"));
        assert_eq!(e.end, at("2026-10-15 11:00"));
        assert_eq!(e.id, "https://p1/cal/abc.ics");
        assert!(!e.recurring);
    }

    #[test]
    fn reads_utc_all_day_durations_and_expanded_occurrences() {
        let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:r\nRECURRENCE-ID:20261012T070000Z\nDTSTART:20261012T070000Z\nDURATION:PT45M\nSUMMARY:Standup\nEND:VEVENT\nBEGIN:VEVENT\nUID:d\nDTSTART;VALUE=DATE:20261020\nSUMMARY:Holiday\nEND:VEVENT\nBEGIN:VEVENT\nUID:x\nSTATUS:CANCELLED\nDTSTART:20261012T090000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let read = events(text, "/c/r.ics", copenhagen());
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].start, at("2026-10-12 09:00"));
        assert_eq!(read[0].end, at("2026-10-12 09:45"));
        assert_eq!(read[0].id, "/c/r.ics#20261012T070000Z");
        assert!(read[0].recurring);
        assert!(read[1].all_day);
        assert_eq!(read[1].end, at("2026-10-21 00:00"));
    }

    #[test]
    fn builds_an_event_a_server_will_take() {
        let text = build(
            "uid-1@niles",
            &NewEvent {
                subject: "Dentist; cleaning".into(),
                start: at("2026-10-15 10:00"),
                end: at("2026-10-15 11:00"),
                all_day: false,
                location: None,
            },
            copenhagen(),
        );
        assert!(text.contains("DTSTART:20261015T080000Z\r\n"));
        assert!(text.contains("SUMMARY:Dentist\\; cleaning\r\n"));
        let back = events(&text, "/x.ics", copenhagen());
        assert_eq!(back[0].start, at("2026-10-15 10:00"));
        assert_eq!(back[0].subject, "Dentist; cleaning");
    }

    #[test]
    fn rewriting_keeps_the_alarm_and_moves_the_time() {
        let moved = rewrite(
            DENTIST,
            &NewEvent {
                subject: "Dentist".into(),
                start: at("2026-10-15 11:00"),
                end: at("2026-10-15 12:00"),
                all_day: false,
                location: Some("Aarhus C".into()),
            },
            copenhagen(),
        );
        assert!(moved.contains("TRIGGER:-PT15M"), "{moved}");
        // The zone's own DTSTART stays; the event has exactly one.
        assert_eq!(moved.matches("DTSTART:2026").count(), 1, "{moved}");
        assert!(moved.contains("DTSTART:19701025T030000"));
        let back = events(&moved, "/x.ics", copenhagen());
        assert_eq!(back[0].start, at("2026-10-15 11:00"));
        assert!(!repeats(&moved));
    }

    #[test]
    fn knows_a_series() {
        assert!(repeats("BEGIN:VEVENT\nRRULE:FREQ=WEEKLY\nEND:VEVENT"));
    }
}

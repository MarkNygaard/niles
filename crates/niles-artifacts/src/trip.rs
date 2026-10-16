//! A trip: days of places and plans between two dates, as JSON.
//!
//! Structured rather than markdown because the app reads it as well as
//! the person: it counts down to the first day, opens today's day while
//! the trip is on, and keeps the packing list as boxes to tick on the
//! way out of the door. Each day is one date with its place, so "what
//! are we doing on Thursday" has one answer, and a booking is marked as
//! one rather than said in a sentence somewhere.
//!
//! Strict, as a training plan is, because the model is the one writing
//! it and reads what is wrong: an unknown field is refused by name
//! rather than dropped, and a link is an https address or nothing, since
//! the app makes it something to tap.

use crate::error::{Error, Result};
use crate::kinds::{Kind, Preview, PreviewLine, Style};
use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

pub(crate) const KIND: Kind = Kind::new(
    "trip",
    "Trip",
    "{\"destination\", \"start\", \"end\", \"travellers\"?, \"image\"?, \"days\": [{\"date\", \
     \"place\", \"title\"?, \"image\"?, \"items\": [{\"time\"?, \"type\", \"title\", \
     \"details\"?, \"location\"?, \"link\"?, \"booked\"?, \"image\"?, \"cost\"?: {\"amount\", \
     \"currency\"}}]}],\"stays\"?: [{\"name\", \
     \"place\", \"check_in\", \"check_out\", \"link\"?, \"booked\"?, \"confirmation\"?}], \
     \"packing\"?: [{\"item\", \"group\"?, \"packed\"?}], \"budget\"?: {\"currency\", \
     \"notes\"?}, \"notes\"?}: a travel plan, day by day. Dates are YYYY-MM-DD; days are in \
     order, one per date, from start to end, and each names its place (the city or area it \
     is spent in). An item's type is travel, stay, activity, food or note; time is \"HH:MM\", \
     and a day's timed items are in order. details is a line or two of plain text, location \
     a name or address on one line, link an https:// address. booked is true only for what \
     is booked. cost is an estimate unless booked, its currency ISO 4217 (\"THB\"). A stay \
     is inside the trip and checks out after it checks in; stays are in the order they \
     begin. travellers are names; packing is a checklist the person ticks (packed), so keep \
     what they ticked when changing the trip. notes is markdown about the whole trip. image \
     is a picture the trip keeps, \"niles-image:12\" (for the trip, a day or an item): \
     artifact_add_image keeps one and answers its name.",
    "Planning",
    validate,
    preview,
);

pub const ITEM_TYPES: &[&str] = &["travel", "stay", "activity", "food", "note"];

/// A year, with a day to spare for a leap one.
const MAX_DAYS: usize = 366;
/// A full day, hour by hour, with room for notes.
const MAX_ITEMS: usize = 30;
const MAX_STAYS: usize = 60;
const MAX_PACKING: usize = 300;
const MAX_TRAVELLERS: usize = 12;
const MAX_NAME: usize = 80;
/// An address with its postcode and country.
const MAX_LOCATION: usize = 200;
const MAX_LINK: usize = 2_000;
const MAX_DETAILS: usize = 1_000;
const MAX_BUDGET_NOTES: usize = 2_000;
const MAX_NOTES: usize = 10_000;
/// The whole trip as JSON: a month of full days with long details is
/// well under it.
const MAX_CHARS: usize = 400_000;
/// In any currency: a year of travel in Indonesian rupiah fits.
const MAX_AMOUNT: f64 = 1_000_000_000.0;

/// Places a card shows, under the line about the trip.
const PREVIEW_PLACES: usize = 3;

const TRIP: &[&str] = &[
    "destination",
    "start",
    "end",
    "travellers",
    "image",
    "days",
    "stays",
    "packing",
    "budget",
    "notes",
];
const DAY: &[&str] = &["date", "place", "title", "image", "items"];
const ITEM: &[&str] = &[
    "time", "type", "title", "details", "location", "link", "booked", "image", "cost",
];
const COST: &[&str] = &["amount", "currency"];
const STAY: &[&str] = &[
    "name",
    "place",
    "check_in",
    "check_out",
    "link",
    "booked",
    "confirmation",
];
const PACKING: &[&str] = &["item", "group", "packed"];
const BUDGET: &[&str] = &["currency", "notes"];

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "trip",
        reason: reason.into(),
    }
}

/// A run that answered with the trip instead of saving it may have put
/// it in a code fence, or a sentence before it.
fn validate(content: &Value) -> Result<Value> {
    match content {
        Value::String(text) => trip(&from_text(text)?),
        other => trip(other),
    }
}

fn from_text(text: &str) -> Result<Value> {
    let text = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(text) {
        return Ok(v);
    }
    match (text.find('{'), text.rfind('}')) {
        (Some(open), Some(close)) if open < close => serde_json::from_str(&text[open..=close])
            .map_err(|e| {
                invalid(format!(
                    "content is a JSON object, and this is not one: {e}"
                ))
            }),
        _ => Err(invalid(
            "content is a JSON object, {\"destination\": …, \"days\": […]}, not text",
        )),
    }
}

fn trip(content: &Value) -> Result<Value> {
    const AT: &str = "the trip";
    let o = fields(content, AT, TRIP)?;
    let destination = name(o, "destination", AT)?
        .ok_or_else(|| invalid("the trip needs \"destination\": \"Thailand\""))?;
    let start =
        date(o, "start", AT)?.ok_or_else(|| invalid("the trip needs \"start\", its first day"))?;
    let end = date(o, "end", AT)?.ok_or_else(|| invalid("the trip needs \"end\", its last day"))?;
    if end < start {
        return Err(invalid(format!(
            "the trip ends on {end}, before it starts on {start}"
        )));
    }
    let length = (end - start).num_days() + 1;
    if length > MAX_DAYS as i64 {
        return Err(invalid(format!(
            "the trip is {length} days, and at most {MAX_DAYS} are kept"
        )));
    }

    let mut out = json!({
        "destination": destination,
        "start": start.to_string(),
        "end": end.to_string(),
    });
    let travellers = list(o, "travellers", AT, MAX_TRAVELLERS)?;
    if !travellers.is_empty() {
        let mut names = Vec::with_capacity(travellers.len());
        for (i, t) in travellers.iter().enumerate() {
            let at = format!("traveller {}", i + 1);
            let line = match t {
                Value::String(s) => one_line(s, &at, "name", MAX_NAME)?,
                _ => return Err(invalid(format!("{at} is a name, such as \"Mark\""))),
            };
            names.extend(line);
        }
        if !names.is_empty() {
            out["travellers"] = json!(names);
        }
    }

    if let Some(image) = image(o, AT)? {
        out["image"] = json!(image);
    }

    let days = present(o, "days")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("the trip needs \"days\", a list of days"))?;
    if days.is_empty() {
        return Err(invalid("the trip has no days"));
    }
    if days.len() > MAX_DAYS {
        return Err(invalid(format!(
            "the trip has {} days, and at most {MAX_DAYS} are kept",
            days.len()
        )));
    }
    let mut kept = Vec::with_capacity(days.len());
    let mut last: Option<NaiveDate> = None;
    for (i, d) in days.iter().enumerate() {
        let (day, when) = day(d, i + 1, start, end)?;
        match last {
            Some(last) if when == last => {
                return Err(invalid(format!(
                    "day {} ({when}) has the same date as the day above it: one day per date, \
                     with everything on it",
                    i + 1
                )));
            }
            Some(last) if when < last => {
                return Err(invalid(format!(
                    "day {} ({when}) comes before the day listed above it: days are in the \
                     order of their dates",
                    i + 1
                )));
            }
            _ => {}
        }
        last = Some(when);
        kept.push(day);
    }
    out["days"] = json!(kept);

    let stays = list(o, "stays", AT, MAX_STAYS)?;
    if !stays.is_empty() {
        let mut kept = Vec::with_capacity(stays.len());
        let mut last: Option<NaiveDate> = None;
        for (i, s) in stays.iter().enumerate() {
            let (stay, check_in) = stay(s, i + 1, start, end)?;
            if last.is_some_and(|last| check_in < last) {
                return Err(invalid(format!(
                    "stay {} ({check_in}) checks in before the stay listed above it: stays are \
                     in the order they begin",
                    i + 1
                )));
            }
            last = Some(check_in);
            kept.push(stay);
        }
        out["stays"] = json!(kept);
    }

    let packing = list(o, "packing", AT, MAX_PACKING)?;
    if !packing.is_empty() {
        let mut kept = Vec::with_capacity(packing.len());
        let mut seen = HashSet::new();
        for (i, p) in packing.iter().enumerate() {
            let thing = packing_item(p, i + 1)?;
            let group = thing["group"].as_str().unwrap_or_default().to_lowercase();
            let item = thing["item"].as_str().unwrap_or_default().to_lowercase();
            if !seen.insert((group, item)) {
                return Err(invalid(format!(
                    "packing item {} ({}) is on the list twice{}: once is enough",
                    i + 1,
                    thing["item"],
                    thing["group"]
                        .as_str()
                        .map(|g| format!(" under {g:?}"))
                        .unwrap_or_default(),
                )));
            }
            kept.push(thing);
        }
        out["packing"] = json!(kept);
    }

    if let Some(b) = present(o, "budget") {
        const AT: &str = "budget";
        let b = fields(b, AT, BUDGET)?;
        let currency = currency(b, AT)?.ok_or_else(|| {
            invalid("budget needs \"currency\", the one its costs are counted in: \"THB\"")
        })?;
        let mut budget = json!({ "currency": currency });
        if let Some(notes) = text(b, "notes", AT, MAX_BUDGET_NOTES)? {
            budget["notes"] = json!(notes);
        }
        out["budget"] = budget;
    }
    if let Some(notes) = text(o, "notes", AT, MAX_NOTES)? {
        out["notes"] = json!(notes);
    }

    let n = out.to_string().len();
    if n > MAX_CHARS {
        return Err(invalid(format!(
            "the trip is {n} characters as JSON, and at most {MAX_CHARS} are kept: shorten \
             the details"
        )));
    }
    Ok(out)
}

/// One day, and its date.
fn day(value: &Value, n: usize, start: NaiveDate, end: NaiveDate) -> Result<(Value, NaiveDate)> {
    let at = format!("day {n}");
    let o = fields(value, &at, DAY)?;
    let when = date(o, "date", &at)?.ok_or_else(|| invalid(format!("{at} needs \"date\"")))?;
    let at = format!("day {n} ({when})");
    if when < start || when > end {
        return Err(invalid(format!(
            "{at} is not in the trip, which runs {start} to {end}"
        )));
    }
    let place = name(o, "place", &at)?.ok_or_else(|| {
        invalid(format!(
            "{at} needs \"place\", where it is spent: \"Bangkok\", or \"Bangkok → Chiang Mai\" \
             on a day of travel"
        ))
    })?;

    let items = list(o, "items", &at, MAX_ITEMS)?;
    let mut kept = Vec::with_capacity(items.len());
    let mut last: Option<String> = None;
    for (i, item) in items.iter().enumerate() {
        let item = day_item(item, &format!("{at}, item {}", i + 1))?;
        if let Some(time) = item["time"].as_str() {
            if last.as_deref().is_some_and(|last| time < last) {
                return Err(invalid(format!(
                    "{at}, item {} ({time}) comes before the timed item above it: a day's \
                     items are in the order of the day",
                    i + 1
                )));
            }
            last = Some(time.to_string());
        }
        kept.push(item);
    }

    let mut out = json!({ "date": when.to_string(), "place": place, "items": kept });
    if let Some(title) = name(o, "title", &at)? {
        out["title"] = json!(title);
    }
    if let Some(image) = image(o, &at)? {
        out["image"] = json!(image);
    }
    Ok((out, when))
}

fn day_item(value: &Value, at: &str) -> Result<Value> {
    let o = fields(value, at, ITEM)?;
    let kind = one_of(o, "type", at, ITEM_TYPES)?
        .ok_or_else(|| invalid(format!("{at} needs \"type\": {}", ITEM_TYPES.join(", "))))?;
    let title = name(o, "title", at)?
        .ok_or_else(|| invalid(format!("{at} needs \"title\": \"Grand Palace\"")))?;
    let mut out = json!({ "type": kind, "title": title });
    if let Some(raw) = string(o, "time", at)? {
        out["time"] = json!(clock(&raw).ok_or_else(|| invalid(format!(
            "{at}: time is HH:MM on a 24-hour clock, such as \"09:30\", not {raw:?}"
        )))?);
    }
    if let Some(details) = text(o, "details", at, MAX_DETAILS)? {
        out["details"] = json!(details);
    }
    if let Some(location) = one_line_field(o, "location", at, MAX_LOCATION)? {
        out["location"] = json!(location);
    }
    if let Some(link) = link(o, "link", at)? {
        out["link"] = json!(link);
    }
    if booked(o, at)? {
        out["booked"] = json!(true);
    }
    if let Some(image) = image(o, at)? {
        out["image"] = json!(image);
    }
    if let Some(c) = present(o, "cost") {
        let at = format!("{at}: cost");
        let c = fields(c, &at, COST)?;
        let amount = amount(c, &at)?
            .ok_or_else(|| invalid(format!("{at} needs \"amount\", a number such as 450")))?;
        let currency = currency(c, &at)?
            .ok_or_else(|| invalid(format!("{at} needs \"currency\", such as \"THB\"")))?;
        out["cost"] = json!({ "amount": amount, "currency": currency });
    }
    Ok(out)
}

/// One stay, and the day it begins.
fn stay(value: &Value, n: usize, start: NaiveDate, end: NaiveDate) -> Result<(Value, NaiveDate)> {
    let at = format!("stay {n}");
    let o = fields(value, &at, STAY)?;
    let stay_name = name(o, "name", &at)?
        .ok_or_else(|| invalid(format!("{at} needs \"name\", the hotel's or the house's")))?;
    let at = format!("stay {n} ({stay_name})");
    let place = name(o, "place", &at)?
        .ok_or_else(|| invalid(format!("{at} needs \"place\": \"Krabi\"")))?;
    let check_in =
        date(o, "check_in", &at)?.ok_or_else(|| invalid(format!("{at} needs \"check_in\"")))?;
    let check_out =
        date(o, "check_out", &at)?.ok_or_else(|| invalid(format!("{at} needs \"check_out\"")))?;
    if check_in < start || check_out > end {
        return Err(invalid(format!(
            "{at} runs {check_in} to {check_out}, and the trip {start} to {end}: a stay is \
             inside the trip"
        )));
    }
    if check_out <= check_in {
        return Err(invalid(format!(
            "{at} checks out on {check_out}, which is not after it checks in on {check_in}"
        )));
    }
    let mut out = json!({
        "name": stay_name,
        "place": place,
        "check_in": check_in.to_string(),
        "check_out": check_out.to_string(),
    });
    if let Some(link) = link(o, "link", &at)? {
        out["link"] = json!(link);
    }
    if booked(o, &at)? {
        out["booked"] = json!(true);
    }
    if let Some(confirmation) = name(o, "confirmation", &at)? {
        out["confirmation"] = json!(confirmation);
    }
    Ok((out, check_in))
}

fn packing_item(value: &Value, n: usize) -> Result<Value> {
    let at = format!("packing item {n}");
    let o = fields(value, &at, PACKING)?;
    let item = name(o, "item", &at)?
        .ok_or_else(|| invalid(format!("{at} needs \"item\": \"Sunscreen\"")))?;
    let packed = match present(o, "packed") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(invalid(format!("{at}: packed is true or false"))),
    };
    let mut out = json!({ "item": item, "packed": packed });
    if let Some(group) = name(o, "group", &at)? {
        out["group"] = json!(group);
    }
    Ok(out)
}

/// An object's fields, refusing any it does not have.
fn fields<'a>(value: &'a Value, at: &str, known: &[&str]) -> Result<&'a Map<String, Value>> {
    let o = value
        .as_object()
        .ok_or_else(|| invalid(format!("{at} is an object, {{\"{}\": …}}", known[0])))?;
    if let Some(k) = o.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(invalid(format!(
            "{at} has {k:?}, which is not part of a trip; its fields are {}",
            known.join(", ")
        )));
    }
    Ok(o)
}

/// A field that is there: `null` is as good as left out.
fn present<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

/// A list of at most `max`, empty when it is left out.
fn list<'a>(o: &'a Map<String, Value>, key: &str, at: &str, max: usize) -> Result<&'a [Value]> {
    let items = match present(o, key) {
        None => return Ok(&[]),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(invalid(format!("{at}: {key} is a list"))),
    };
    if items.len() > max {
        return Err(invalid(format!(
            "{at}: {key} has {}, and at most {max} are kept",
            items.len()
        )));
    }
    Ok(items)
}

fn string(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    match present(o, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(format!("{at}: {key} is text"))),
    }
}

/// A name or a title: one line of at most [`MAX_NAME`] characters.
fn name(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    one_line_field(o, key, at, MAX_NAME)
}

fn one_line_field(
    o: &Map<String, Value>,
    key: &str,
    at: &str,
    max: usize,
) -> Result<Option<String>> {
    match string(o, key, at)? {
        None => Ok(None),
        Some(raw) => one_line(&raw, at, key, max),
    }
}

/// `raw` on one line, or nothing when it is empty.
fn one_line(raw: &str, at: &str, key: &str, max: usize) -> Result<Option<String>> {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        return Err(invalid(format!(
            "{at}: {key} is longer than {max} characters"
        )));
    }
    Ok((!line.is_empty()).then_some(line))
}

/// Text that may run to lines, tidied as a document's is.
fn text(o: &Map<String, Value>, key: &str, at: &str, max: usize) -> Result<Option<String>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let tidy: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let tidy = tidy.trim();
    let n = tidy.chars().count();
    if n > max {
        return Err(invalid(format!(
            "{at}: {key} is {n} characters, and at most {max} are kept"
        )));
    }
    Ok((!tidy.is_empty()).then(|| tidy.to_string()))
}

fn one_of(
    o: &Map<String, Value>,
    key: &str,
    at: &str,
    allowed: &[&'static str],
) -> Result<Option<&'static str>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let raw = raw.trim().to_lowercase();
    allowed
        .iter()
        .find(|a| **a == raw)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            invalid(format!(
                "{at}: {key} is one of {}, not {raw:?}",
                allowed.join(", ")
            ))
        })
}

fn date(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<NaiveDate>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let raw = raw.trim();
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .filter(|_| raw.len() == 10)
        .map(Some)
        .ok_or_else(|| invalid(format!("{at}: {key} is a date, YYYY-MM-DD, not {raw:?}")))
}

fn booked(o: &Map<String, Value>, at: &str) -> Result<bool> {
    match present(o, "booked") {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(invalid(format!("{at}: booked is true or false"))),
    }
}

/// What a cost comes to: 0 for what is free, to two decimals. Written as
/// text it is still the number.
fn amount(o: &Map<String, Value>, at: &str) -> Result<Option<f64>> {
    let Some(v) = present(o, "amount") else {
        return Ok(None);
    };
    let x = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| invalid(format!("{at}: amount is a number, such as 450")))?;
    if !(0.0..=MAX_AMOUNT).contains(&x) {
        return Err(invalid(format!(
            "{at}: amount is {x}, and it is at least 0 and at most {MAX_AMOUNT}"
        )));
    }
    Ok(Some((x * 100.0).round() / 100.0))
}

/// Three letters, as ISO 4217 has them: "THB", "DKK".
fn currency(o: &Map<String, Value>, at: &str) -> Result<Option<String>> {
    let Some(raw) = string(o, "currency", at)? else {
        return Ok(None);
    };
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(Some(code))
    } else {
        Err(invalid(format!(
            "{at}: currency is its three-letter ISO 4217 code, such as \"THB\" or \"DKK\", not \
             {raw:?}"
        )))
    }
}

/// An https address to somewhere, as the app will open it: never
/// `javascript:`, a page on this machine, or a login tucked in front of
/// the host.
fn link(o: &Map<String, Value>, key: &str, at: &str) -> Result<Option<String>> {
    let Some(raw) = string(o, key, at)? else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let refused = || {
        invalid(format!(
            "{at}: {key} is an https:// address, such as \"https://www.tourismthailand.org/\", \
             not {raw:?}"
        ))
    };
    if raw.len() > MAX_LINK || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refused());
    }
    let url = url::Url::parse(raw).map_err(|_| refused())?;
    let named = matches!(url.host(), Some(url::Host::Domain(d)) if d.contains('.'));
    if url.scheme() != "https" || !named || !url.username().is_empty() || url.password().is_some() {
        return Err(refused());
    }
    Ok(Some(url.to_string()))
}

/// A picture the trip keeps, by its name, `niles-image:12`; its id alone
/// is taken as the name. Never an address: the app shows only what
/// Niles keeps. Whether the trip still keeps it is not asked here (see
/// [`crate::images`]).
fn image(o: &Map<String, Value>, at: &str) -> Result<Option<String>> {
    let refused = |what: &Value| {
        invalid(format!(
            "{at}: image is a picture the trip keeps, \"niles-image:12\", as artifact_add_image \
             answers it, not {what}"
        ))
    };
    let named = |id: i64| Some(format!("{}{id}", crate::images::SCHEME));
    match present(o, "image") {
        None => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(v @ Value::String(s)) => crate::images::parse_reference(s)
            .map(named)
            .ok_or_else(|| refused(v)),
        Some(v @ Value::Number(n)) => n
            .as_i64()
            .filter(|id| *id > 0)
            .map(named)
            .ok_or_else(|| refused(v)),
        Some(v) => Err(refused(v)),
    }
}

/// "9:30" or "09:30", kept as "09:30".
fn clock(raw: &str) -> Option<String> {
    let (h, m) = raw.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// The trip and where it goes, from its first day. Not from today: the
/// preview is kept with the version it was made from, and a card read a
/// month later would still say what "next" was then. Unlike a training
/// plan's sessions nothing on a trip is marked off as it passes, so
/// there is nothing to move it on by; the route from the start is the
/// line that stays true.
fn preview(content: &Value) -> Preview {
    let mut head = Vec::new();
    if let Some(destination) = content["destination"].as_str() {
        head.push(destination.to_string());
    }
    let start = content["start"].as_str().and_then(parse_date);
    let end = content["end"].as_str().and_then(parse_date);
    if let (Some(start), Some(end)) = (start, end) {
        head.push(span(start, end));
        head.push(match (end - start).num_days() + 1 {
            1 => "1 day".to_string(),
            n => format!("{n} days"),
        });
    }
    let mut lines = Vec::new();
    if !head.is_empty() {
        lines.push(PreviewLine {
            text: head.join(" · "),
            style: Style::Heading,
            checked: None,
        });
    }
    let days = content["days"].as_array().map_or(&[][..], Vec::as_slice);
    let mut route: Vec<(&str, NaiveDate, NaiveDate)> = Vec::new();
    for d in days {
        let (Some(place), Some(when)) =
            (d["place"].as_str(), d["date"].as_str().and_then(parse_date))
        else {
            continue;
        };
        match route.last_mut() {
            Some((last, _, to)) if *last == place => *to = when,
            _ => route.push((place, when, when)),
        }
    }
    lines.extend(
        route
            .into_iter()
            .take(PREVIEW_PLACES)
            .map(|(place, from, to)| PreviewLine {
                text: format!("{} · {place}", span(from, to)),
                style: Style::Bullet,
                checked: None,
            }),
    );
    Preview { lines }
}

/// "3 Jan", "3–6 Jan", or "30 Jan – 2 Feb" across a month.
fn span(from: NaiveDate, to: NaiveDate) -> String {
    if from == to {
        from.format("%-d %b").to_string()
    } else if (from.year(), from.month()) == (to.year(), to.month()) {
        format!("{}–{}", from.day(), to.format("%-d %b"))
    } else {
        format!("{} – {}", from.format("%-d %b"), to.format("%-d %b"))
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight days: Bangkok, then the north, then the islands.
    fn trip_json() -> Value {
        json!({
            "destination": " Thailand ",
            "start": "2027-01-03",
            "end": "2027-01-10",
            "travellers": ["Mark", "  Majse "],
            "days": [
                { "date": "2027-01-03", "place": "Bangkok", "title": "Arrive", "items": [
                    { "time": "7:05", "type": "travel", "title": "Land at BKK", "booked": true,
                      "cost": { "amount": "5400.5", "currency": "dkk" } },
                    { "type": "note", "title": "Get a SIM at the airport" },
                    { "time": "19:30", "type": "Food", "title": "Dinner at Jay Fai",
                      "location": "327 Maha Chai Rd,\n Bangkok", "link": "https://www.jayfai.com/",
                      "details": "Queue early.\r\nCash only." }
                ]},
                { "date": "2027-01-04", "place": "Bangkok", "items": [
                    { "time": "09:00", "type": "activity", "title": "Grand Palace",
                      "cost": { "amount": 500, "currency": "THB" } }
                ]},
                { "date": "2027-01-05", "place": "Bangkok → Chiang Mai", "items": [] },
                { "date": "2027-01-06", "place": "Chiang Mai" },
                { "date": "2027-01-08", "place": "Koh Lanta", "items": [] }
            ],
            "stays": [
                { "name": "Riva Surya", "place": "Bangkok", "check_in": "2027-01-03",
                  "check_out": "2027-01-05", "booked": true, "confirmation": "AB123" },
                { "name": "Cha Cha Lanta", "place": "Koh Lanta", "check_in": "2027-01-08",
                  "check_out": "2027-01-10", "link": "https://example.com/stay" }
            ],
            "packing": [
                { "item": "Sunscreen", "group": "Toiletries" },
                { "item": "Passport", "packed": true },
                { "item": "Sunscreen", "group": "Beach bag" }
            ],
            "budget": { "currency": "thb", "notes": "About 3,000 a day for two." },
            "notes": "Assumed you **fly** in to Bangkok."
        })
    }

    fn refused(content: Value) -> String {
        validate(&content).unwrap_err().to_string()
    }

    #[test]
    fn a_trip_is_kept_with_its_values_tidied() {
        let v = validate(&trip_json()).unwrap();
        assert_eq!(v["destination"], "Thailand");
        assert_eq!(v["travellers"], json!(["Mark", "Majse"]));
        let arrive = &v["days"][0];
        assert_eq!(arrive["title"], "Arrive");
        assert_eq!(arrive["items"][0]["time"], "07:05");
        assert_eq!(arrive["items"][0]["booked"], true);
        assert_eq!(
            arrive["items"][0]["cost"],
            json!({ "amount": 5400.5, "currency": "DKK" })
        );
        assert!(arrive["items"][1].get("booked").is_none());
        let dinner = &arrive["items"][2];
        assert_eq!(dinner["type"], "food");
        assert_eq!(dinner["location"], "327 Maha Chai Rd, Bangkok");
        assert_eq!(dinner["details"], "Queue early.\nCash only.");
        assert_eq!(v["days"][3]["items"], json!([]));
        assert_eq!(v["stays"][0]["confirmation"], "AB123");
        assert_eq!(v["packing"][0]["packed"], false);
        assert_eq!(v["packing"][1]["packed"], true);
        assert_eq!(v["budget"]["currency"], "THB");
    }

    #[test]
    fn a_kept_trip_is_kept_again_unchanged() {
        let once = validate(&trip_json()).unwrap();
        assert_eq!(validate(&once).unwrap(), once);
    }

    #[test]
    fn a_trip_answered_as_text_is_read_out_of_it() {
        let text = format!("Here it is:\n```json\n{}\n```", trip_json());
        let v = validate(&Value::String(text)).unwrap();
        assert_eq!(v["destination"], "Thailand");
        assert!(refused(json!("Two weeks in Thailand.")).contains("JSON object"));
    }

    #[test]
    fn an_unknown_field_is_refused_by_name() {
        let mut t = trip_json();
        t["days"][1]["items"][0]["price"] = json!(500);
        let e = refused(t);
        assert!(
            e.contains("day 2 (2027-01-04), item 1 has \"price\""),
            "{e}"
        );
        assert!(e.contains("cost"), "{e}");

        let mut t = trip_json();
        t["hotels"] = json!([]);
        assert!(refused(t).contains("stays"));
    }

    #[test]
    fn null_is_the_same_as_left_out() {
        let mut t = trip_json();
        t["notes"] = Value::Null;
        t["stays"] = Value::Null;
        t["days"][0]["items"][0]["time"] = Value::Null;
        let v = validate(&t).unwrap();
        assert!(v.get("notes").is_none());
        assert!(v.get("stays").is_none());
        assert!(v["days"][0]["items"][0].get("time").is_none());
    }

    #[test]
    fn a_trip_needs_where_when_and_a_day() {
        let mut t = trip_json();
        t.as_object_mut().unwrap().remove("destination");
        assert!(refused(t).contains("destination"));
        let mut t = trip_json();
        t.as_object_mut().unwrap().remove("end");
        assert!(refused(t).contains("its last day"));
        let mut t = trip_json();
        t["days"] = json!([]);
        assert!(refused(t).contains("no days"));
        let mut t = trip_json();
        t["end"] = json!("2027-01-02");
        assert!(refused(t).contains("before it starts"));
        let mut t = trip_json();
        t["end"] = json!("2028-01-10");
        assert!(refused(t).contains("at most 366"));
        assert!(refused(json!(42)).contains("object"));
    }

    #[test]
    fn days_are_inside_the_trip_in_order_one_per_date() {
        let mut t = trip_json();
        t["days"][4]["date"] = json!("2027-01-11");
        let e = refused(t);
        assert!(e.contains("runs 2027-01-03 to 2027-01-10"), "{e}");

        let mut t = trip_json();
        t["days"][1]["date"] = json!("2027-01-03");
        assert!(refused(t).contains("one day per date"));

        let mut t = trip_json();
        t["days"][3]["date"] = json!("2027-01-04");
        assert!(refused(t).contains("order of their dates"));

        let mut t = trip_json();
        t["days"][2].as_object_mut().unwrap().remove("place");
        assert!(refused(t).contains("needs \"place\""));
    }

    #[test]
    fn a_day_s_timed_items_are_in_the_order_of_the_day() {
        let mut t = trip_json();
        t["days"][0]["items"][2]["time"] = json!("06:00");
        let e = refused(t);
        assert!(e.contains("item 3 (06:00) comes before"), "{e}");
    }

    #[test]
    fn times_types_costs_and_currencies_are_checked() {
        for bad in ["7", "24:00", "7:5", "noon", "07:05:00"] {
            let mut t = trip_json();
            t["days"][0]["items"][0]["time"] = json!(bad);
            assert!(refused(t).contains("HH:MM"), "{bad}");
        }
        let mut t = trip_json();
        t["days"][0]["items"][1]["type"] = json!("shopping");
        assert!(refused(t).contains("travel, stay, activity, food, note"));
        let mut t = trip_json();
        t["days"][1]["items"][0]["cost"]["amount"] = json!(-1);
        assert!(refused(t).contains("at least 0"));
        let mut t = trip_json();
        t["days"][1]["items"][0]["cost"]["currency"] = json!("baht");
        assert!(refused(t).contains("ISO 4217"));
        let mut t = trip_json();
        t["days"][1]["items"][0]["cost"] = json!({ "amount": 500 });
        assert!(refused(t).contains("needs \"currency\""));
        let mut t = trip_json();
        t["days"][0]["items"][0]["booked"] = json!("yes");
        assert!(refused(t).contains("true or false"));
    }

    #[test]
    fn a_link_is_an_https_address_and_nothing_else() {
        for bad in [
            "javascript:alert(1)",
            "http://www.jayfai.com/",
            "https://localhost/",
            "https://127.0.0.1/",
            "https://user:pw@evil.example/",
            "data:text/html,hi",
            "www.jayfai.com",
            "https://jay fai.com",
        ] {
            let mut t = trip_json();
            t["days"][0]["items"][2]["link"] = json!(bad);
            assert!(refused(t).contains("https:// address"), "{bad}");
        }
        let mut t = trip_json();
        t["stays"][1]["link"] = json!("HTTPS://Example.com/stay");
        let v = validate(&t).unwrap();
        assert_eq!(v["stays"][1]["link"], "https://example.com/stay");
    }

    #[test]
    fn a_stay_is_inside_the_trip_and_checks_out_after_it_checks_in() {
        let mut t = trip_json();
        t["stays"][1]["check_out"] = json!("2027-01-11");
        let e = refused(t);
        assert!(e.contains("stay 2 (Cha Cha Lanta)"), "{e}");
        assert!(e.contains("inside the trip"), "{e}");

        let mut t = trip_json();
        t["stays"][0]["check_out"] = json!("2027-01-03");
        assert!(refused(t).contains("not after it checks in"));

        let mut t = trip_json();
        t["stays"][1]["check_in"] = json!("2027-01-03");
        t["stays"][1]["check_out"] = json!("2027-01-04");
        t["stays"][0]["check_in"] = json!("2027-01-04");
        assert!(refused(t).contains("order they begin"));
    }

    #[test]
    fn a_packing_item_is_on_the_list_once_per_group() {
        let mut t = trip_json();
        t["packing"][2]["group"] = json!("toiletries");
        let e = refused(t);
        assert!(e.contains("twice under \"toiletries\""), "{e}");

        let mut t = trip_json();
        t["packing"][1]["packed"] = json!("yes");
        assert!(refused(t).contains("true or false"));
    }

    #[test]
    fn a_runaway_trip_is_refused() {
        let mut t = trip_json();
        let note = json!({ "type": "note", "title": "Rest" });
        t["days"][2]["items"] = json!(vec![note; MAX_ITEMS + 1]);
        assert!(refused(t).contains("at most 30"));

        let mut t = trip_json();
        t["packing"] = json!(
            (0..=MAX_PACKING)
                .map(|i| json!({ "item": format!("thing {i}") }))
                .collect::<Vec<_>>()
        );
        assert!(refused(t).contains("at most 300"));

        let mut t = trip_json();
        t["notes"] = json!("a".repeat(MAX_NOTES + 1));
        assert!(refused(t).contains("at most 10000"));

        let mut t = trip_json();
        t["days"][0]["title"] = json!("a".repeat(MAX_NAME + 1));
        assert!(refused(t).contains("longer than 80"));
    }

    #[test]
    fn the_preview_is_the_trip_and_its_route_from_the_start() {
        let v = validate(&trip_json()).unwrap();
        let lines: Vec<(Style, String)> = preview(&v)
            .lines
            .into_iter()
            .map(|l| (l.style, l.text))
            .collect();
        assert_eq!(
            lines,
            vec![
                (Style::Heading, "Thailand · 3–10 Jan · 8 days".into()),
                (Style::Bullet, "3–4 Jan · Bangkok".into()),
                (Style::Bullet, "5 Jan · Bangkok → Chiang Mai".into()),
                (Style::Bullet, "6 Jan · Chiang Mai".into()),
            ]
        );
    }

    #[test]
    fn a_picture_is_one_the_trip_keeps_by_its_name() {
        let mut t = trip_json();
        t["image"] = json!(" niles-image:3 ");
        t["days"][0]["image"] = json!(4);
        t["days"][0]["items"][2]["image"] = json!("niles-image:5");
        t["days"][1]["image"] = json!("");
        let v = validate(&t).unwrap();
        assert_eq!(v["image"], "niles-image:3");
        assert_eq!(v["days"][0]["image"], "niles-image:4");
        assert_eq!(v["days"][0]["items"][2]["image"], "niles-image:5");
        assert!(v["days"][1].get("image").is_none());
        assert_eq!(validate(&v).unwrap(), v);

        for bad in [
            json!("https://upload.wikimedia.org/palace.jpg"),
            json!("data:image/png;base64,iVBORw0KGgo="),
            json!("niles-image:x"),
            json!(-2),
            json!({ "id": 3 }),
        ] {
            let mut t = trip_json();
            t["days"][0]["items"][0]["image"] = bad.clone();
            let e = refused(t);
            assert!(
                e.contains("day 1 (2027-01-03), item 1: image"),
                "{bad}: {e}"
            );
            assert!(e.contains("artifact_add_image"), "{e}");
        }
    }

    #[test]
    fn a_span_across_a_month_names_both() {
        let d = |s| parse_date(s).unwrap();
        assert_eq!(span(d("2027-01-30"), d("2027-02-02")), "30 Jan – 2 Feb");
        assert_eq!(span(d("2026-12-28"), d("2027-01-04")), "28 Dec – 4 Jan");
    }
}

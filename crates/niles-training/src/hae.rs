//! What the Health Auto Export iOS app posts with its REST API
//! automation, read into workouts and days.
//!
//! The body is `{"data": {"metrics": [...], "workouts": [...]}}`. Each
//! automation sends one data type, so either array may be missing, and
//! `data` can carry others (symptoms, ECG, medications, state of mind)
//! which are not training and are left alone. Each metric is
//! `{name, units, data: [{date, qty, source}]}`, its units on the metric
//! and chosen by the person in the app; sleep has its own shape.
//! Workouts come in two export versions: v1, with no `id` or
//! `duration`, `activeEnergy` as one quantity and `elevation: {ascent}`;
//! and v2, the current one, with an `id`, `duration` in seconds,
//! `activeEnergyBurned`, `heartRate: {min, avg, max}` and `elevationUp`,
//! where `activeEnergy` is the per-minute samples instead. Both are read.
//!
//! Lenient by design: a metric Niles has no use for is named in
//! [`Batch::ignored_metrics`] and otherwise skipped, and one reading
//! that will not parse costs that reading, not the batch. The phone
//! sends the same days again on its next sync anyway.

use crate::model::{Activity, Kind, Wellness};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The `source` every workout from here is kept under.
pub const SOURCE: &str = "health_auto_export";

#[derive(Debug, Default, PartialEq)]
pub struct Batch {
    pub activities: Vec<Activity>,
    /// One per day, oldest first, holding only what this post said.
    pub wellness: Vec<Wellness>,
    /// Metric names in the post that are not training data.
    pub ignored_metrics: Vec<String>,
    /// Readings and workouts that were there but could not be read.
    pub unreadable: usize,
}

/// Which field of the day a metric feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    RestingHr,
    Hrv,
    Sleep,
    Weight,
    BodyFat,
}

impl Field {
    fn of(name: &str) -> Option<Field> {
        match name {
            "resting_heart_rate" => Some(Field::RestingHr),
            // SDNN, which is what Apple Health keeps.
            "heart_rate_variability" | "heart_rate_variability_sdnn" => Some(Field::Hrv),
            "sleep_analysis" => Some(Field::Sleep),
            // The help pages print `weight_&_body_mass`; the app sends
            // `weight_body_mass`.
            "weight_body_mass" | "weight_&_body_mass" | "body_mass" => Some(Field::Weight),
            "body_fat_percentage" => Some(Field::BodyFat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Reading {
    at: DateTime<Utc>,
    /// In canonical units.
    value: f64,
    /// For a stretch of sleep from the unsummarised export, which the
    /// night is the sum of, the source it came from. Every other reading
    /// is whole on its own.
    segment_of: Option<String>,
}

/// Read a post. Only a body that is not the export at all is an error.
///
/// `tz` is the house's zone, for a timestamp without an offset.
pub fn parse<Tz: TimeZone>(body: &Value, tz: &Tz) -> Result<Batch, String> {
    let data = body
        .get("data")
        .and_then(Value::as_object)
        .ok_or("expected Health Auto Export JSON: an object with \"data\"")?;
    let mut batch = Batch::default();

    let mut readings: BTreeMap<(NaiveDate, Field), Vec<Reading>> = BTreeMap::new();
    let mut ignored = BTreeSet::new();
    for metric in data
        .get("metrics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = metric.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(field) = Field::of(name) else {
            if !name.is_empty() {
                ignored.insert(name.to_string());
            }
            continue;
        };
        let units = metric.get("units").and_then(Value::as_str).unwrap_or("");
        for point in metric
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match reading(field, units, point, tz) {
                Ok(Some((day, r))) => readings.entry((day, field)).or_default().push(r),
                Ok(None) => {}
                Err(()) => batch.unreadable += 1,
            }
        }
    }

    let mut days: BTreeMap<NaiveDate, Wellness> = BTreeMap::new();
    for ((day, field), mut values) in readings {
        values.sort_by_key(|r| r.at);
        let value = match field {
            Field::Sleep => night(&values),
            // A weighing is a moment: the latest that day stands.
            Field::Weight | Field::BodyFat => values.last().map_or(0.0, |r| r.value),
            Field::RestingHr | Field::Hrv => {
                values.iter().map(|r| r.value).sum::<f64>() / values.len() as f64
            }
        };
        let w = days.entry(day).or_insert_with(|| Wellness::on(day));
        let slot = match field {
            Field::RestingHr => &mut w.resting_hr,
            Field::Hrv => &mut w.hrv_ms,
            Field::Sleep => &mut w.sleep_s,
            Field::Weight => &mut w.weight_kg,
            Field::BodyFat => &mut w.body_fat_pct,
        };
        *slot = Some(value);
    }
    batch.wellness = days.into_values().filter(|w| !w.is_empty()).collect();

    for workout in data
        .get("workouts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match activity(workout, tz) {
            Some(a) => batch.activities.push(a),
            None => batch.unreadable += 1,
        }
    }
    batch.ignored_metrics = ignored.into_iter().collect();
    Ok(batch)
}

/// One night from what was said about it. Two sleeps on one day are
/// usually the same night from two sources, the phone and the watch:
/// the longer is the one that tracked it. Stretches add up per source
/// first, for the same reason.
fn night(readings: &[Reading]) -> f64 {
    let mut per_source: BTreeMap<&str, f64> = BTreeMap::new();
    let mut whole: f64 = 0.0;
    for r in readings {
        match &r.segment_of {
            Some(source) => *per_source.entry(source.as_str()).or_default() += r.value,
            None => whole = whole.max(r.value),
        }
    }
    per_source.into_values().fold(whole, f64::max)
}

/// One reading and the day it belongs to. `Ok(None)` for one that is
/// fine but not wanted, such as time awake, and `Err` for one that
/// could not be read.
fn reading<Tz: TimeZone>(
    field: Field,
    units: &str,
    point: &Value,
    tz: &Tz,
) -> Result<Option<(NaiveDate, Reading)>, ()> {
    if field == Field::Sleep && point.get("startDate").is_some() {
        return sleep_segment(units, point, tz);
    }
    let (day, at) = point
        .get("date")
        .and_then(Value::as_str)
        .and_then(|d| when(d, tz))
        .ok_or(())?;
    let qty = || point.get("qty").and_then(number).ok_or(());
    let value = match field {
        Field::RestingHr | Field::Hrv => qty()?,
        Field::Sleep => sleep_seconds(units, point).ok_or(())?,
        Field::Weight => kilograms(qty()?, units).ok_or(())?,
        Field::BodyFat => {
            // Apple Health keeps a fraction, and the export has sent it
            // both ways. Nobody is one per cent fat, so at most 1 is a
            // fraction.
            let qty = qty()?;
            if qty <= 1.0 { qty * 100.0 } else { qty }
        }
    };
    if !(value.is_finite() && value > 0.0) {
        return Err(());
    }
    let reading = Reading {
        at,
        value,
        segment_of: None,
    };
    Ok(Some((day, reading)))
}

/// A stretch of the night from the unsummarised export: `startDate`,
/// `endDate`, `qty` in hours and `value` naming the stage. The night is
/// the day it ended on, the morning woken up in.
fn sleep_segment<Tz: TimeZone>(
    units: &str,
    point: &Value,
    tz: &Tz,
) -> Result<Option<(NaiveDate, Reading)>, ()> {
    let stage = point
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("Asleep");
    if !matches!(stage, "Asleep" | "Core" | "Deep" | "REM" | "Unspecified") {
        return Ok(None);
    }
    let time = |k: &str| {
        point
            .get(k)
            .and_then(Value::as_str)
            .and_then(|t| when(t, tz))
    };
    let (_, start) = time("startDate").ok_or(())?;
    let (day, end) = time("endDate").ok_or(())?;
    let seconds = match point.get("qty").and_then(number) {
        Some(qty) => in_seconds(qty, units),
        None => (end - start).num_seconds() as f64,
    };
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err(());
    }
    let source = point.get("source").and_then(Value::as_str).unwrap_or("");
    let reading = Reading {
        at: end,
        value: seconds,
        segment_of: Some(source.to_string()),
    };
    Ok(Some((day, reading)))
}

/// The night's sleep in seconds. The summarised export gives each stage
/// in hours, and `totalSleep` in newer versions. `asleep` is only sleep
/// no stage was given to, so a watch that tracks stages leaves it at
/// zero and puts the night in core, deep and REM.
fn sleep_seconds(units: &str, point: &Value) -> Option<f64> {
    let get = |k: &str| point.get(k).and_then(number);
    let total = get("totalSleep").filter(|t| *t > 0.0).or_else(|| {
        let sum = ["asleep", "core", "deep", "rem"]
            .iter()
            .filter_map(|k| get(k))
            .filter(|h| *h > 0.0)
            .sum::<f64>();
        (sum > 0.0).then_some(sum)
    })?;
    Some(in_seconds(total, units))
}

fn in_seconds(qty: f64, units: &str) -> f64 {
    match units {
        "min" => qty * 60.0,
        "s" | "sec" => qty,
        _ => qty * 3600.0,
    }
}

fn activity<Tz: TimeZone>(w: &Value, tz: &Tz) -> Option<Activity> {
    let name = w
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Workout")
        .trim();
    let (_, started_at) = when(w.get("start").and_then(Value::as_str)?, tz)?;
    let ended_at = w
        .get("end")
        .and_then(Value::as_str)
        .and_then(|e| when(e, tz))
        .map(|(_, at)| at);

    // v1 has no duration: the time between start and end.
    let duration_s = w
        .get("duration")
        .and_then(number)
        .filter(|d| *d > 0.0)
        .or_else(|| {
            let secs = (ended_at? - started_at).num_seconds() as f64;
            (secs > 0.0).then_some(secs)
        });
    let distance_m = w
        .get("distance")
        .and_then(quantity)
        .and_then(|(q, u)| metres(q, u.as_deref().unwrap_or("km")));
    let kcal =
        |v: &Value| quantity(v).and_then(|(q, u)| kilocalories(q, u.as_deref().unwrap_or("kcal")));
    let energy_kcal = match w
        .get("activeEnergyBurned")
        .or_else(|| w.get("activeEnergy"))
    {
        // v2's `activeEnergy` is the per-minute samples, sent only when
        // the workout's metrics are; v2 always has the total beside it.
        Some(Value::Array(samples)) => {
            let total: f64 = samples.iter().filter_map(kcal).sum();
            (total > 0.0).then_some(total)
        }
        Some(total) => kcal(total),
        None => None,
    };
    let heart = w.get("heartRate");
    let bpm = |v: Option<&Value>| v.and_then(number).filter(|h| *h > 0.0);
    let avg_hr = bpm(heart
        .and_then(|h| h.get("avg"))
        .or_else(|| w.get("avgHeartRate")));
    let max_hr = bpm(heart
        .and_then(|h| h.get("max"))
        .or_else(|| w.get("maxHeartRate")));
    let elevation_gain_m = match w.get("elevationUp") {
        Some(up) => quantity(up).and_then(|(q, u)| metres(q, u.as_deref().unwrap_or("m"))),
        None => w.get("elevation").and_then(|e| {
            let units = e.get("units").and_then(Value::as_str).unwrap_or("m");
            metres(e.get("ascent").and_then(number)?, units)
        }),
    };

    let external_id = match w.get("id").and_then(Value::as_str).map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        // v1 has no id: one start and one kind of workout are one
        // workout, however often it is sent.
        _ => Sha256::digest(format!("{}|{name}", started_at.to_rfc3339()).as_bytes())
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect(),
    };

    Some(Activity {
        source: SOURCE.into(),
        external_id,
        started_at,
        kind: Kind::from_workout_name(name),
        name: name.to_string(),
        distance_m: distance_m.filter(|d| *d > 0.0),
        duration_s,
        moving_s: None,
        avg_hr,
        max_hr,
        elevation_gain_m,
        energy_kcal,
        raw: w.clone(),
    })
}

/// "2026-10-14 07:12:00 +0200", as the app writes it; the same in
/// 12-hour time, which iOS uses when the phone is set to it; RFC 3339;
/// or a bare date, as summarised sleep has. The day is the one written,
/// the phone's own: a night slept abroad stays the night it was there.
/// Without an offset, the house's zone.
fn when<Tz: TimeZone>(text: &str, tz: &Tz) -> Option<(NaiveDate, DateTime<Utc>)> {
    let text = text.trim().replace(['\u{202f}', '\u{a0}'], " ");
    let with_offset = DateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S %z")
        .or_else(|_| DateTime::parse_from_str(&text, "%Y-%m-%d %I:%M:%S %p %z"))
        .or_else(|_| DateTime::parse_from_rfc3339(&text));
    if let Ok(at) = with_offset {
        return Some((at.date_naive(), at.to_utc()));
    }
    let naive = NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| {
            NaiveDate::parse_from_str(&text, "%Y-%m-%d").map(|d| d.and_time(NaiveTime::MIN))
        })
        .ok()?;
    let at = tz.from_local_datetime(&naive).earliest()?.to_utc();
    Some((naive.date(), at))
}

/// A number, or a `{qty}` around one: v1 and v2 differ on which.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Object(o) => o.get("qty").and_then(number),
        _ => None,
    }
}

/// `{qty, units}`, or a bare number with no units.
fn quantity(v: &Value) -> Option<(f64, Option<String>)> {
    let units = v.get("units").and_then(Value::as_str).map(str::to_string);
    Some((number(v)?, units))
}

fn metres(qty: f64, units: &str) -> Option<f64> {
    let factor = match units.trim().to_ascii_lowercase().as_str() {
        "km" => 1000.0,
        "m" => 1.0,
        "mi" | "mile" | "miles" => 1609.344,
        "yd" | "yard" | "yards" => 0.9144,
        "ft" | "foot" | "feet" => 0.3048,
        _ => return None,
    };
    Some(qty * factor)
}

fn kilograms(qty: f64, units: &str) -> Option<f64> {
    let factor = match units.trim().to_ascii_lowercase().as_str() {
        "kg" => 1.0,
        "lb" | "lbs" => 0.453_592_37,
        "g" => 0.001,
        "st" | "stone" => 6.350_293_18,
        _ => return None,
    };
    Some(qty * factor)
}

fn kilocalories(qty: f64, units: &str) -> Option<f64> {
    match units.trim().to_ascii_lowercase().as_str() {
        "kcal" | "cal" => Some(qty),
        "kj" => Some(qty / 4.184),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TZ: chrono_tz::Tz = chrono_tz::Europe::Copenhagen;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "v2-metric" => include_str!("../fixtures/hae-v2-metric.json"),
            "v2-imperial" => include_str!("../fixtures/hae-v2-imperial.json"),
            "v1" => include_str!("../fixtures/hae-v1.json"),
            "sleep-segments" => include_str!("../fixtures/hae-sleep-segments.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 0.01)
    }

    #[test]
    fn a_v2_run_in_kilometres_is_read_in_canonical_units() {
        let batch = parse(&fixture("v2-metric"), &TZ).unwrap();
        let run = &batch.activities[0];
        assert_eq!(run.external_id, "550E8400-E29B-41D4-A716-446655440000");
        assert_eq!(run.kind, Kind::Run);
        assert_eq!(run.name, "Outdoor Run");
        assert_eq!(run.started_at.to_rfc3339(), "2026-10-14T05:00:00+00:00");
        assert!(close(run.distance_m, 5020.0));
        assert!(close(run.duration_s, 1920.4));
        assert_eq!(run.avg_hr, Some(148.0));
        assert_eq!(run.max_hr, Some(171.0));
        assert_eq!(run.elevation_gain_m, Some(42.0));
        assert_eq!(run.energy_kcal, Some(350.0));
        assert!(run.raw.get("route").is_some(), "the route is kept");
    }

    #[test]
    fn miles_feet_kilojoules_and_pounds_become_metres_kilocalories_and_kilograms() {
        let batch = parse(&fixture("v2-imperial"), &TZ).unwrap();
        let run = &batch.activities[0];
        assert_eq!(run.kind, Kind::Run);
        assert!(close(run.distance_m, 13.1 * 1609.344));
        assert!(close(run.elevation_gain_m, 120.0 * 0.3048));
        let walk = &batch.activities[1];
        assert_eq!(walk.kind, Kind::Walk);
        assert!(close(walk.energy_kcal, 820.0 / 4.184));
        let weighed = batch
            .wellness
            .iter()
            .find(|w| w.weight_kg.is_some())
            .unwrap();
        assert!(close(weighed.weight_kg, 176.4 * 0.453_592_37));
        // Sent as a fraction: a percent here.
        assert!(close(weighed.body_fat_pct, 24.0));
    }

    #[test]
    fn the_day_s_heart_sleep_and_scale_are_one_day() {
        let batch = parse(&fixture("v2-metric"), &TZ).unwrap();
        let today = batch
            .wellness
            .iter()
            .find(|w| w.date == day(2026, 10, 14))
            .unwrap();
        assert_eq!(today.resting_hr, Some(54.0));
        assert_eq!(today.hrv_ms, Some(48.6));
        assert!(close(today.sleep_s, 7.1 * 3600.0));
        assert_eq!(today.weight_kg, Some(61.2));
        assert_eq!(today.body_fat_pct, Some(23.4));
    }

    #[test]
    fn metrics_that_are_not_training_are_named_and_skipped() {
        let batch = parse(&fixture("v2-metric"), &TZ).unwrap();
        assert_eq!(batch.ignored_metrics, ["heart_rate", "step_count"]);
        assert_eq!(batch.unreadable, 0);
    }

    #[test]
    fn a_v1_workout_has_no_id_and_gets_a_stable_one() {
        let first = parse(&fixture("v1"), &TZ).unwrap();
        let again = parse(&fixture("v1"), &TZ).unwrap();
        let run = &first.activities[0];
        assert_eq!(run.external_id, again.activities[0].external_id);
        assert_eq!(run.external_id.len(), 32);
        assert!(close(run.duration_s, 47.0 * 60.0), "from start to end");
        assert!(close(run.distance_m, 8040.0));
        assert_eq!(run.avg_hr, Some(151.0));
        assert_eq!(run.max_hr, Some(176.0));
        assert_eq!(run.elevation_gain_m, Some(31.0));
        assert_eq!(run.energy_kcal, Some(512.0));
        // Stage-tracked sleep from an older app: asleep is zero, and the
        // night is the stages.
        assert!(close(batch_sleep(&first, day(2026, 10, 12)), 6.5 * 3600.0));
    }

    fn batch_sleep(batch: &Batch, on: NaiveDate) -> Option<f64> {
        batch.wellness.iter().find(|w| w.date == on)?.sleep_s
    }

    #[test]
    fn unsummarised_sleep_adds_up_the_night_and_not_both_sources() {
        let batch = parse(&fixture("sleep-segments"), &TZ).unwrap();
        // Core 3h + deep 1h + REM 1.5h from the watch; the phone saw
        // less of the same night. Awake and in bed are not sleep.
        assert!(close(batch_sleep(&batch, day(2026, 10, 14)), 5.5 * 3600.0));
        assert_eq!(batch.wellness.len(), 1);
    }

    #[test]
    fn several_readings_on_a_day_become_one() {
        let body = json!({"data": {"metrics": [
            {"name": "heart_rate_variability", "units": "ms", "data": [
                {"date": "2026-10-14 02:00:00 +0200", "qty": 40},
                {"date": "2026-10-14 04:00:00 +0200", "qty": 50}
            ]},
            {"name": "weight_body_mass", "units": "kg", "data": [
                {"date": "2026-10-14 21:00:00 +0200", "qty": 61.9},
                {"date": "2026-10-14 07:00:00 +0200", "qty": 61.0}
            ]}
        ]}});
        let batch = parse(&body, &TZ).unwrap();
        assert_eq!(batch.wellness[0].hrv_ms, Some(45.0), "averaged");
        assert_eq!(batch.wellness[0].weight_kg, Some(61.9), "the latest");
    }

    #[test]
    fn twelve_hour_time_and_bare_dates_are_read() {
        let (d, at) = when("2026-10-14 7:05:00\u{202f}PM +0200", &TZ).unwrap();
        assert_eq!(d, day(2026, 10, 14));
        assert_eq!(at.to_rfc3339(), "2026-10-14T17:05:00+00:00");
        let (d, at) = when("2026-10-14", &TZ).unwrap();
        assert_eq!(d, day(2026, 10, 14));
        assert_eq!(at.to_rfc3339(), "2026-10-13T22:00:00+00:00");
    }

    #[test]
    fn a_day_is_the_one_the_phone_wrote() {
        // Just after midnight in Tokyo is still the evening before at
        // home; the morning's weighing belongs to Tokyo's day.
        let (d, _) = when("2026-10-15 00:30:00 +0900", &TZ).unwrap();
        assert_eq!(d, day(2026, 10, 15));
    }

    #[test]
    fn a_bad_reading_costs_itself_and_not_the_batch() {
        let body = json!({"data": {
            "metrics": [{"name": "resting_heart_rate", "units": "count/min", "data": [
                {"date": "yesterday", "qty": 50},
                {"date": "2026-10-14 00:00:00 +0200", "qty": 52}
            ]}],
            "workouts": [{"name": "Outdoor Run"}]
        }});
        let batch = parse(&body, &TZ).unwrap();
        assert_eq!(batch.unreadable, 2);
        assert_eq!(batch.wellness[0].resting_hr, Some(52.0));
    }

    #[test]
    fn a_body_that_is_not_the_export_is_refused() {
        assert!(parse(&json!({"hello": 1}), &TZ).is_err());
        let empty = parse(&json!({"data": {}}), &TZ).unwrap();
        assert_eq!(empty, Batch::default());
    }
}

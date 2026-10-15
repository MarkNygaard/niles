//! "How was my week?" worked out here rather than by the model, which
//! adds up distances and averages paces wrongly often enough to tell
//! somebody they ran a marathon on Sunday.

use crate::model::{Activity, Kind};
use chrono::{NaiveDate, TimeZone};
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub runs: usize,
    pub run_distance_m: f64,
    pub run_seconds: f64,
    /// The day and distance of the longest run.
    pub longest_run: Option<(NaiveDate, f64)>,
    /// Over the runs with both a distance and a time.
    pub pace_s_per_km: Option<f64>,
    /// Every workout, runs included.
    pub activities: usize,
}

/// The period `from..=to` (house days), from the person's workouts.
/// Anything outside it is ignored.
pub fn summarise<Tz: TimeZone>(
    activities: &[Activity],
    from: NaiveDate,
    to: NaiveDate,
    tz: &Tz,
) -> Summary {
    let day_of = |a: &Activity| a.started_at.with_timezone(tz).date_naive();
    let within: Vec<&Activity> = activities
        .iter()
        .filter(|a| (from..=to).contains(&day_of(a)))
        .collect();
    let runs: Vec<&Activity> = within
        .iter()
        .copied()
        .filter(|a| a.kind == Kind::Run)
        .collect();

    let paced: Vec<(f64, f64)> = runs
        .iter()
        .filter_map(|a| Some((a.distance_m.filter(|d| *d > 0.0)?, a.pace_seconds()?)))
        .collect();
    let paced_metres: f64 = paced.iter().map(|(m, _)| m).sum();
    let pace_s_per_km = (paced_metres > 0.0)
        .then(|| paced.iter().map(|(_, s)| s).sum::<f64>() / (paced_metres / 1000.0));

    let longest_run = runs
        .iter()
        .filter_map(|a| Some((day_of(a), a.distance_m?)))
        .max_by(|a, b| a.1.total_cmp(&b.1));

    Summary {
        from,
        to,
        runs: runs.len(),
        run_distance_m: runs.iter().filter_map(|a| a.distance_m).sum(),
        run_seconds: runs.iter().filter_map(|a| a.duration_s).sum(),
        longest_run,
        pace_s_per_km,
        activities: within.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn run(started: &str, km: f64, minutes: f64) -> Activity {
        Activity {
            source: "strava".into(),
            external_id: started.into(),
            started_at: at(started),
            kind: Kind::Run,
            name: "Morning Run".into(),
            distance_m: Some(km * 1000.0),
            duration_s: Some(minutes * 60.0),
            moving_s: None,
            avg_hr: None,
            max_hr: None,
            elevation_gain_m: None,
            energy_kcal: None,
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn a_week_of_runs_adds_up() {
        let tz = chrono_tz::Europe::Copenhagen;
        let mut ride = run("2026-10-10T08:00:00Z", 40.0, 90.0);
        ride.kind = Kind::Ride;
        let activities = [
            run("2026-10-08T06:00:00Z", 5.0, 30.0),
            run("2026-10-11T07:00:00Z", 10.0, 55.0),
            ride,
            // The Monday before, just outside the week.
            run("2026-10-07T21:30:00Z", 8.0, 48.0),
        ];
        let s = summarise(&activities, day(8), day(14), &tz);
        assert_eq!(s.runs, 2);
        assert_eq!(s.activities, 3);
        assert_eq!(s.run_distance_m, 15_000.0);
        assert_eq!(s.longest_run, Some((day(11), 10_000.0)));
        // 85 minutes over 15 km: 5:40 a kilometre.
        assert_eq!(s.pace_s_per_km.map(f64::round), Some(340.0));
    }

    #[test]
    fn a_run_late_in_the_evening_is_on_the_house_s_day() {
        let tz = chrono_tz::Europe::Copenhagen;
        // 22:30 UTC on the 7th is 00:30 on the 8th in Copenhagen.
        let activities = [run("2026-10-07T22:30:00Z", 5.0, 30.0)];
        let s = summarise(&activities, day(8), day(14), &tz);
        assert_eq!(s.runs, 1);
    }

    #[test]
    fn a_run_without_a_distance_counts_but_has_no_pace() {
        let tz = chrono_tz::UTC;
        let mut treadmill = run("2026-10-09T06:00:00Z", 0.0, 30.0);
        treadmill.distance_m = None;
        let s = summarise(&[treadmill], day(8), day(14), &tz);
        assert_eq!(s.runs, 1);
        assert_eq!(s.pace_s_per_km, None);
        assert_eq!(s.longest_run, None);
    }
}

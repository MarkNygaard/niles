//! "How was my week?" worked out here rather than by the model, which
//! adds up distances and averages paces wrongly often enough to tell
//! somebody they ran a marathon on Sunday.

use crate::model::{Activity, Kind, Wellness};
use chrono::{NaiveDate, TimeZone};
use serde::Serialize;

/// An average over the period and over the same number of days before
/// it, so "lower" and "higher" mean something.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Trend {
    pub average: Option<f64>,
    pub previous_average: Option<f64>,
    /// Days with a reading in the period.
    pub days: usize,
}

impl Trend {
    pub fn change(&self) -> Option<f64> {
        Some(self.average? - self.previous_average?)
    }
}

/// The first and last weighing in the period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct WeightChange {
    pub first: Option<(NaiveDate, f64)>,
    pub last: Option<(NaiveDate, f64)>,
}

impl WeightChange {
    pub fn change_kg(&self) -> Option<f64> {
        let (first, last) = (self.first?, self.last?);
        (first.0 != last.0).then_some(last.1 - first.1)
    }
}

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
    pub resting_hr: Trend,
    pub hrv_ms: Trend,
    pub sleep_s: Trend,
    pub weight: WeightChange,
}

/// The period `from..=to` (house days), from the person's workouts and
/// days. `wellness` should reach back as far before `from` as the period
/// is long, for the trends; anything outside either is ignored.
pub fn summarise<Tz: TimeZone>(
    activities: &[Activity],
    wellness: &[Wellness],
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

    let length = (to - from).num_days() + 1;
    let before = (
        from - chrono::Duration::days(length),
        from - chrono::Duration::days(1),
    );
    let trend = |field: fn(&Wellness) -> Option<f64>| {
        let average = |lo: NaiveDate, hi: NaiveDate| {
            let values: Vec<f64> = wellness
                .iter()
                .filter(|w| w.date >= lo && w.date <= hi)
                .filter_map(field)
                .collect();
            let n = values.len();
            ((n > 0).then(|| values.iter().sum::<f64>() / n as f64), n)
        };
        let (now, days) = average(from, to);
        let (then, _) = average(before.0, before.1);
        Trend {
            average: now,
            previous_average: then,
            days,
        }
    };

    let mut weighed = wellness
        .iter()
        .filter(|w| w.date >= from && w.date <= to)
        .filter_map(|w| Some((w.date, w.weight_kg?)));
    let first = weighed.next();
    let last = weighed.last().or(first);

    Summary {
        from,
        to,
        runs: runs.len(),
        run_distance_m: runs.iter().filter_map(|a| a.distance_m).sum(),
        run_seconds: runs.iter().filter_map(|a| a.duration_s).sum(),
        longest_run,
        pace_s_per_km,
        activities: within.len(),
        resting_hr: trend(|w| w.resting_hr),
        hrv_ms: trend(|w| w.hrv_ms),
        sleep_s: trend(|w| w.sleep_s),
        weight: WeightChange { first, last },
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
            source: "test".into(),
            external_id: started.into(),
            started_at: at(started),
            kind: Kind::Run,
            name: "Outdoor Run".into(),
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

    fn rested(d: u32, hr: f64) -> Wellness {
        Wellness {
            resting_hr: Some(hr),
            ..Wellness::on(day(d))
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
        let s = summarise(&activities, &[], day(8), day(14), &tz);
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
        let s = summarise(&activities, &[], day(8), day(14), &tz);
        assert_eq!(s.runs, 1);
    }

    #[test]
    fn a_run_without_a_distance_counts_but_has_no_pace() {
        let tz = chrono_tz::UTC;
        let mut treadmill = run("2026-10-09T06:00:00Z", 0.0, 30.0);
        treadmill.distance_m = None;
        let s = summarise(&[treadmill], &[], day(8), day(14), &tz);
        assert_eq!(s.runs, 1);
        assert_eq!(s.pace_s_per_km, None);
        assert_eq!(s.longest_run, None);
    }

    #[test]
    fn trends_compare_with_the_period_before() {
        let tz = chrono_tz::UTC;
        let wellness = [
            rested(2, 56.0),
            rested(4, 54.0),
            rested(9, 52.0),
            rested(12, 50.0),
            // After the period: ignored.
            rested(15, 70.0),
        ];
        let s = summarise(&[], &wellness, day(8), day(14), &tz);
        assert_eq!(s.resting_hr.average, Some(51.0));
        assert_eq!(s.resting_hr.previous_average, Some(55.0));
        assert_eq!(s.resting_hr.change(), Some(-4.0));
        assert_eq!(s.resting_hr.days, 2);
        assert_eq!(s.hrv_ms, Trend::default());
    }

    #[test]
    fn the_weight_change_is_last_weighing_less_first() {
        let tz = chrono_tz::UTC;
        let weighed = |d: u32, kg: f64| Wellness {
            weight_kg: Some(kg),
            ..Wellness::on(day(d))
        };
        let s = summarise(
            &[],
            &[weighed(8, 62.0), rested(10, 50.0), weighed(13, 61.4)],
            day(8),
            day(14),
            &tz,
        );
        assert_eq!(s.weight.first, Some((day(8), 62.0)));
        assert_eq!(s.weight.last, Some((day(13), 61.4)));
        assert!((s.weight.change_kg().unwrap() + 0.6).abs() < 1e-9);

        let once = summarise(&[], &[weighed(9, 62.0)], day(8), day(14), &tz);
        assert_eq!(once.weight.change_kg(), None, "one weighing is no change");
    }
}

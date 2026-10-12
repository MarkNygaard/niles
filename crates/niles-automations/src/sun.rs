//! Sunrise and sunset at the house, from its latitude and longitude.
//!
//! The sunrise equation as the NOAA tables use it, good to a minute or
//! two away from the poles — closer than anybody standing at a window
//! could say the sun went down.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use std::f64::consts::PI;

/// Where the house is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

impl Location {
    /// The configured position, unless it is the unset `0, 0` — a spot
    /// in the Gulf of Guinea nobody lives at.
    pub fn of(latitude: f64, longitude: f64) -> Option<Self> {
        let set = latitude != 0.0 || longitude != 0.0;
        (set && latitude.abs() <= 90.0 && longitude.abs() <= 180.0).then_some(Self {
            latitude,
            longitude,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunEvent {
    Sunrise,
    Sunset,
}

impl SunEvent {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sunrise" => Some(SunEvent::Sunrise),
            "sunset" => Some(SunEvent::Sunset),
            _ => None,
        }
    }
}

/// When the sun rises and sets on `day` (a date at the house), or
/// `None` where it does neither that day.
pub fn times(day: NaiveDate, at: Location) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let rad = |deg: f64| deg * PI / 180.0;
    let deg = |rad: f64| rad * 180.0 / PI;
    // Days since noon on 1 January 2000, at the house's longitude.
    let julian_day = f64::from(day.num_days_from_ce()) + 1_721_424.5 + 0.5;
    let n = (julian_day - 2_451_545.0 + 0.0008).round();
    let mean_noon = n - at.longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_noon).rem_euclid(360.0);
    let centre = 1.9148 * rad(anomaly).sin()
        + 0.0200 * rad(2.0 * anomaly).sin()
        + 0.0003 * rad(3.0 * anomaly).sin();
    let longitude = (anomaly + centre + 180.0 + 102.9372).rem_euclid(360.0);
    let transit =
        2_451_545.0 + mean_noon + 0.0053 * rad(anomaly).sin() - 0.0069 * rad(2.0 * longitude).sin();
    let declination = (rad(longitude).sin() * rad(23.4397).sin()).asin();
    let latitude = rad(at.latitude);
    let cos_hour = (rad(-0.833).sin() - latitude.sin() * declination.sin())
        / (latitude.cos() * declination.cos());
    if !(-1.0..=1.0).contains(&cos_hour) {
        return None;
    }
    let hour = deg(cos_hour.acos());
    let instant = |julian: f64| {
        let seconds = ((julian - 2_440_587.5) * 86_400.0).round() as i64;
        Utc.timestamp_opt(seconds, 0).single()
    };
    Some((
        instant(transit - hour / 360.0)?,
        instant(transit + hour / 360.0)?,
    ))
}

/// When `event`, moved by `offset_minutes`, falls on `day` at the house.
pub fn at(
    event: SunEvent,
    offset_minutes: i32,
    day: NaiveDate,
    location: Location,
    tz: Tz,
) -> Option<DateTime<Tz>> {
    let (rise, set) = times(day, location)?;
    let base = match event {
        SunEvent::Sunrise => rise,
        SunEvent::Sunset => set,
    };
    Some((base + chrono::Duration::minutes(i64::from(offset_minutes))).with_timezone(&tz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn aarhus() -> Location {
        Location::of(56.1572, 10.2107).unwrap()
    }

    fn minutes(t: DateTime<Tz>) -> i64 {
        i64::from(t.hour()) * 60 + i64::from(t.minute())
    }

    #[test]
    fn rises_and_sets_over_aarhus_in_october() {
        // sunrise-sunset.org: 12 October 2026, sunrise 07:42, sunset 18:28 (CEST).
        let tz: Tz = "Europe/Copenhagen".parse().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let rise = at(SunEvent::Sunrise, 0, day, aarhus(), tz).unwrap();
        let set = at(SunEvent::Sunset, 0, day, aarhus(), tz).unwrap();
        assert!((minutes(rise) - (7 * 60 + 42)).abs() <= 2, "{rise}");
        assert!((minutes(set) - (18 * 60 + 28)).abs() <= 2, "{set}");
        let before = at(SunEvent::Sunset, -30, day, aarhus(), tz).unwrap();
        assert_eq!(minutes(set) - minutes(before), 30);
    }

    #[test]
    fn midsummer_is_long_and_midwinter_short() {
        let length = |m: u32, d: u32| {
            let (rise, set) =
                times(NaiveDate::from_ymd_opt(2026, m, d).unwrap(), aarhus()).unwrap();
            (set - rise).num_minutes()
        };
        assert!(length(6, 21) > 17 * 60);
        assert!(length(12, 21) < 7 * 60);
    }

    #[test]
    fn an_unset_location_is_none() {
        assert_eq!(Location::of(0.0, 0.0), None);
        assert!(Location::of(56.1, 10.2).is_some());
    }

    #[test]
    fn the_midnight_sun_never_sets() {
        let tromso = Location::of(69.65, 18.96).unwrap();
        assert_eq!(
            times(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), tromso),
            None
        );
    }
}

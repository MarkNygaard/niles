//! Open-Meteo data models.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::Serialize;

/// Measurement system for the forecast request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Metric,
    Imperial,
}

/// A geocoded location.
#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub name: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
}

/// Current weather conditions.
#[derive(Debug, Clone, Serialize)]
pub struct CurrentConditions {
    pub temperature: f64,
    pub weather_code: u16,
    pub weather_description: String,
    pub wind_speed: f64,
    pub relative_humidity: u8,
}

/// A single day in the forecast.
#[derive(Debug, Clone, Serialize)]
pub struct DailyForecast {
    pub date: NaiveDate,
    pub weather_code: u16,
    pub weather_description: String,
    pub temperature_max: f64,
    pub temperature_min: f64,
    pub sunrise: NaiveTime,
    pub sunset: NaiveTime,
    pub precipitation_sum: f64,
}

/// Complete weather report for a location.
#[derive(Debug, Clone, Serialize)]
pub struct WeatherReport {
    pub location: Location,
    pub units: Units,
    pub current: Option<CurrentConditions>,
    pub daily: Vec<DailyForecast>,
    pub observed_at: DateTime<Utc>,
}

/// Convert a WMO weather code to a human-readable description.
pub fn weather_code_to_description(code: u16) -> &'static str {
    match code {
        0 => "clear sky",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 => "fog",
        48 => "depositing rime fog",
        51 => "light drizzle",
        53 => "moderate drizzle",
        55 => "dense drizzle",
        56 => "light freezing drizzle",
        57 => "dense freezing drizzle",
        61 => "slight rain",
        63 => "moderate rain",
        65 => "heavy rain",
        66 => "light freezing rain",
        67 => "heavy freezing rain",
        71 => "slight snow fall",
        73 => "moderate snow fall",
        75 => "heavy snow fall",
        77 => "snow grains",
        80 => "slight rain showers",
        81 => "moderate rain showers",
        82 => "violent rain showers",
        85 => "slight snow showers",
        86 => "heavy snow showers",
        95 => "thunderstorm",
        96 => "thunderstorm with slight hail",
        99 => "thunderstorm with heavy hail",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn units_serializes_lowercase() {
        assert_eq!(json!(Units::Metric), "metric");
        assert_eq!(json!(Units::Imperial), "imperial");
    }

    #[test]
    fn weather_code_clear_sky() {
        assert_eq!(weather_code_to_description(0), "clear sky");
    }

    #[test]
    fn weather_code_thunderstorm() {
        assert_eq!(weather_code_to_description(95), "thunderstorm");
    }

    #[test]
    fn weather_code_unknown() {
        assert_eq!(weather_code_to_description(999), "unknown");
    }

    #[test]
    fn weather_report_round_trips() {
        let report = WeatherReport {
            location: Location {
                name: Some("Aarhus".into()),
                latitude: 56.0,
                longitude: 10.0,
                country: Some("Denmark".into()),
            },
            units: Units::Metric,
            current: Some(CurrentConditions {
                temperature: 15.0,
                weather_code: 1,
                weather_description: "mainly clear".into(),
                wind_speed: 3.5,
                relative_humidity: 65,
            }),
            daily: vec![DailyForecast {
                date: NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
                weather_code: 2,
                weather_description: "partly cloudy".into(),
                temperature_max: 18.0,
                temperature_min: 12.0,
                sunrise: NaiveTime::from_hms_opt(5, 30, 0).unwrap(),
                sunset: NaiveTime::from_hms_opt(21, 45, 0).unwrap(),
                precipitation_sum: 0.0,
            }],
            observed_at: Utc::now(),
        };
        let v = serde_json::to_value(&report).unwrap();
        assert!(v.get("location").is_some());
        assert!(v.get("units").is_some());
        assert!(v.get("current").is_some());
        assert!(v.get("daily").is_some());
    }
}

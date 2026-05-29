//! Open-Meteo HTTP client.

use crate::error::{Error, Result};
use crate::model::{
    CurrentConditions, DailyForecast, Location, Units, WeatherReport, weather_code_to_description,
};
use crate::transport::{HttpTransport, WeatherTransport};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

pub const GEOCODING_BASE_URL: &str = "https://geocoding-api.open-meteo.com";

/// Client configuration.
#[derive(Debug, Clone)]
pub struct OpenMeteoConfig {
    pub base_url: String,
    pub geocoding_base_url: String,
    pub request_timeout: Duration,
    pub user_agent: String,
}

impl Default for OpenMeteoConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.open-meteo.com".into(),
            geocoding_base_url: GEOCODING_BASE_URL.into(),
            request_timeout: Duration::from_secs(10),
            user_agent: concat!(
                "niles/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/MarkNygaard/niles)"
            )
            .into(),
        }
    }
}

/// Open-Meteo forecast + geocoding client.
pub struct OpenMeteoClient {
    transport: Arc<dyn WeatherTransport>,
    config: OpenMeteoConfig,
}

impl OpenMeteoClient {
    /// Create a client using the default [`HttpTransport`].
    pub fn new(config: OpenMeteoConfig) -> Result<Self> {
        let transport = Arc::new(HttpTransport::new(
            &config.user_agent,
            config.request_timeout,
        ));
        Ok(Self::with_transport(transport, config))
    }

    /// Create a client with a custom transport (useful for testing).
    pub fn with_transport(transport: Arc<dyn WeatherTransport>, config: OpenMeteoConfig) -> Self {
        Self { transport, config }
    }

    /// Geocode a place name to a [`Location`].
    pub async fn geocode(&self, query: &str) -> Result<Location> {
        let url = format!(
            "{}/v1/search?name={}&count=1&language=en&format=json",
            self.config.geocoding_base_url,
            urlencoding::encode(query),
        );

        let body = self.transport.get(&url).await?;
        let parsed: GeoResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        let rows = parsed.results.ok_or_else(|| Error::GeocodeEmpty {
            query: query.into(),
        })?;

        let row = rows.into_iter().next().ok_or_else(|| Error::GeocodeEmpty {
            query: query.into(),
        })?;

        Ok(Location {
            name: Some(row.name),
            latitude: row.latitude,
            longitude: row.longitude,
            country: row.country,
        })
    }

    /// Fetch a weather forecast for the given location.
    pub async fn fetch_forecast(
        &self,
        location: &Location,
        days: u8,
        units: Units,
        include_current: bool,
    ) -> Result<WeatherReport> {
        if days == 0 {
            return Err(Error::InvalidInput {
                reason: "days must be >= 1".into(),
            });
        }
        let days = days.min(16);

        let (temp_unit, wind_unit, precip_unit) = match units {
            Units::Metric => ("celsius", "ms", "mm"),
            Units::Imperial => ("fahrenheit", "mph", "inch"),
        };

        let mut url = format!(
            "{}/v1/forecast?latitude={}&longitude={}&daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,precipitation_sum&forecast_days={}&temperature_unit={}&wind_speed_unit={}&precipitation_unit={}&timezone=auto",
            self.config.base_url,
            location.latitude,
            location.longitude,
            days,
            temp_unit,
            wind_unit,
            precip_unit,
        );

        if include_current {
            url.push_str(
                "&current=temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m",
            );
        }

        let body = self.transport.get(&url).await?;
        let resp: ForecastResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        let observed_at = if let Some(ref current) = resp.current {
            parse_observed_at(&current.time, resp.utc_offset_seconds)?
        } else {
            Utc::now()
        };

        let current = resp.current.map(|c| CurrentConditions {
            temperature: c.temperature,
            weather_code: c.weather_code,
            weather_description: weather_code_to_description(c.weather_code).to_string(),
            wind_speed: c.wind_speed,
            relative_humidity: c.relative_humidity,
        });

        let daily = build_daily(resp.daily)?;

        Ok(WeatherReport {
            location: location.clone(),
            units,
            current,
            daily,
            observed_at,
        })
    }
}

#[derive(Deserialize)]
struct GeoResp {
    results: Option<Vec<GeoRow>>,
}

#[derive(Deserialize)]
struct GeoRow {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
}

#[derive(Deserialize)]
struct ForecastResp {
    #[serde(rename = "utc_offset_seconds")]
    utc_offset_seconds: i32,
    current: Option<CurrentResp>,
    daily: DailyResp,
}

#[derive(Deserialize)]
struct CurrentResp {
    time: String,
    #[serde(rename = "temperature_2m")]
    temperature: f64,
    #[serde(rename = "relative_humidity_2m")]
    relative_humidity: u8,
    #[serde(rename = "weather_code")]
    weather_code: u16,
    #[serde(rename = "wind_speed_10m")]
    wind_speed: f64,
}

#[derive(Deserialize)]
struct DailyResp {
    time: Vec<String>,
    #[serde(rename = "weather_code")]
    weather_code: Vec<u16>,
    #[serde(rename = "temperature_2m_max")]
    temperature_2m_max: Vec<f64>,
    #[serde(rename = "temperature_2m_min")]
    temperature_2m_min: Vec<f64>,
    sunrise: Vec<String>,
    sunset: Vec<String>,
    #[serde(rename = "precipitation_sum")]
    precipitation_sum: Vec<f64>,
}

fn parse_observed_at(time_str: &str, offset_seconds: i32) -> Result<DateTime<Utc>> {
    let naive =
        NaiveDateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M").map_err(|e| Error::Parse {
            reason: format!("invalid current.time '{time_str}': {e}"),
        })?;
    let offset = FixedOffset::east_opt(offset_seconds).ok_or_else(|| Error::Parse {
        reason: format!("invalid utc_offset_seconds {offset_seconds}"),
    })?;
    let dt = offset
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| Error::Parse {
            reason: format!("ambiguous local time '{time_str}'"),
        })?;
    Ok(dt.to_utc())
}

fn build_daily(daily: DailyResp) -> Result<Vec<DailyForecast>> {
    let n = daily.time.len();
    if daily.weather_code.len() != n
        || daily.temperature_2m_max.len() != n
        || daily.temperature_2m_min.len() != n
        || daily.sunrise.len() != n
        || daily.sunset.len() != n
        || daily.precipitation_sum.len() != n
    {
        return Err(Error::Parse {
            reason: "daily array lengths do not match".into(),
        });
    }

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let date =
            NaiveDate::parse_from_str(&daily.time[i], "%Y-%m-%d").map_err(|e| Error::Parse {
                reason: format!("invalid daily.time '{}': {e}", daily.time[i]),
            })?;
        let sunrise = parse_time(&daily.sunrise[i])?;
        let sunset = parse_time(&daily.sunset[i])?;
        let code = daily.weather_code[i];
        out.push(DailyForecast {
            date,
            weather_code: code,
            weather_description: weather_code_to_description(code).to_string(),
            temperature_max: daily.temperature_2m_max[i],
            temperature_min: daily.temperature_2m_min[i],
            sunrise,
            sunset,
            precipitation_sum: daily.precipitation_sum[i],
        });
    }
    Ok(out)
}

fn parse_time(s: &str) -> Result<NaiveTime> {
    let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").map_err(|e| Error::Parse {
        reason: format!("invalid time '{s}': {e}"),
    })?;
    Ok(dt.time())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_url: Arc<Mutex<Option<String>>>,
        response: Arc<Mutex<Option<Result<String>>>>,
    }

    impl MockTransport {
        fn new(response: Result<String>) -> Self {
            Self {
                last_url: Arc::new(Mutex::new(None)),
                response: Arc::new(Mutex::new(Some(response))),
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl WeatherTransport for MockTransport {
        async fn get(&self, url: &str) -> Result<String> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            self.response
                .lock()
                .unwrap()
                .take()
                .expect("mock called more than once")
        }
    }

    fn client_with(response: Result<String>) -> (MockTransport, OpenMeteoClient) {
        let mock = MockTransport::new(response);
        let client =
            OpenMeteoClient::with_transport(Arc::new(mock.clone()), OpenMeteoConfig::default());
        (mock, client)
    }

    fn forecast_json() -> String {
        r#"{
            "utc_offset_seconds": 7200,
            "current": {
                "time": "2024-06-01T14:00",
                "temperature_2m": 15.3,
                "relative_humidity_2m": 65,
                "weather_code": 1,
                "wind_speed_10m": 3.5
            },
            "daily": {
                "time": ["2024-06-01", "2024-06-02"],
                "weather_code": [1, 2],
                "temperature_2m_max": [18.0, 19.5],
                "temperature_2m_min": [12.0, 13.0],
                "sunrise": ["2024-06-01T05:30", "2024-06-02T05:29"],
                "sunset": ["2024-06-01T21:45", "2024-06-02T21:46"],
                "precipitation_sum": [0.0, 2.5]
            }
        }"#
        .into()
    }

    fn geocode_json() -> String {
        r#"{
            "results": [
                {
                    "name": "Aarhus",
                    "latitude": 56.1567,
                    "longitude": 10.2108,
                    "country": "Denmark"
                }
            ]
        }"#
        .into()
    }

    #[tokio::test]
    async fn fetch_forecast_happy_path() {
        let (_, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: Some("Aarhus".into()),
            latitude: 56.0,
            longitude: 10.0,
            country: Some("Denmark".into()),
        };
        let report = client
            .fetch_forecast(&loc, 2, Units::Metric, true)
            .await
            .unwrap();
        assert_eq!(report.location.latitude, 56.0);
        assert!(report.current.is_some());
        let current = report.current.unwrap();
        assert_eq!(current.temperature, 15.3);
        assert_eq!(current.weather_code, 1);
        assert_eq!(current.weather_description, "mainly clear");
        assert_eq!(current.wind_speed, 3.5);
        assert_eq!(current.relative_humidity, 65);
        assert_eq!(report.daily.len(), 2);
        assert_eq!(report.daily[0].temperature_max, 18.0);
        assert_eq!(report.daily[0].temperature_min, 12.0);
        assert_eq!(report.daily[0].weather_description, "mainly clear");
        assert_eq!(report.daily[1].weather_description, "partly cloudy");
    }

    #[tokio::test]
    async fn fetch_forecast_observed_at_utc() {
        let (_, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: None,
            latitude: 56.0,
            longitude: 10.0,
            country: None,
        };
        let report = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap();
        // 2024-06-01T14:00 +02:00 -> 2024-06-01T12:00 UTC
        assert_eq!(report.observed_at.to_rfc3339(), "2024-06-01T12:00:00+00:00");
    }

    #[tokio::test]
    async fn geocode_happy_path() {
        let (_, client) = client_with(Ok(geocode_json()));
        let loc = client.geocode("Aarhus").await.unwrap();
        assert_eq!(loc.name, Some("Aarhus".into()));
        assert_eq!(loc.latitude, 56.1567);
        assert_eq!(loc.longitude, 10.2108);
        assert_eq!(loc.country, Some("Denmark".into()));
    }

    #[tokio::test]
    async fn geocode_empty_results() {
        let (_, client) = client_with(Ok(r#"{"results": []}"#.into()));
        let err = client.geocode("Nowhere").await.unwrap_err();
        assert!(matches!(err, Error::GeocodeEmpty { query } if query == "Nowhere"));
    }

    #[tokio::test]
    async fn geocode_no_results_field() {
        let (_, client) = client_with(Ok(r#"{}"#.into()));
        let err = client.geocode("Nowhere").await.unwrap_err();
        assert!(matches!(err, Error::GeocodeEmpty { query } if query == "Nowhere"));
    }

    #[tokio::test]
    async fn fetch_forecast_days_zero() {
        let (_, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 0, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput { reason } if reason.contains("days must be >= 1"))
        );
    }

    #[tokio::test]
    async fn fetch_forecast_days_clamped_to_16() {
        let (mock, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        client
            .fetch_forecast(&loc, 100, Units::Metric, true)
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("forecast_days=16"));
        assert!(!url.contains("forecast_days=100"));
    }

    #[tokio::test]
    async fn bad_status_propagates() {
        let (_, client) = client_with(Err(Error::BadStatus {
            status: 500,
            body: "oops".into(),
        }));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 500, body } if body == "oops"));
    }

    #[tokio::test]
    async fn bad_json_propagates() {
        let (_, client) = client_with(Ok("not json".into()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[tokio::test]
    async fn include_current_false_omits_param() {
        let (mock, client) = client_with(Ok(forecast_json_no_current()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let report = client
            .fetch_forecast(&loc, 1, Units::Metric, false)
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(!url.contains("current="));
        assert!(report.current.is_none());
    }

    fn forecast_json_no_current() -> String {
        r#"{
            "utc_offset_seconds": 0,
            "daily": {
                "time": ["2024-06-01"],
                "weather_code": [0],
                "temperature_2m_max": [20.0],
                "temperature_2m_min": [10.0],
                "sunrise": ["2024-06-01T06:00"],
                "sunset": ["2024-06-01T20:00"],
                "precipitation_sum": [0.0]
            }
        }"#
        .into()
    }

    #[tokio::test]
    async fn units_imperial_in_url() {
        let (mock, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        client
            .fetch_forecast(&loc, 1, Units::Imperial, true)
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("temperature_unit=fahrenheit"));
        assert!(url.contains("wind_speed_unit=mph"));
        assert!(url.contains("precipitation_unit=inch"));
    }

    #[tokio::test]
    async fn units_metric_in_url() {
        let (mock, client) = client_with(Ok(forecast_json()));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("temperature_unit=celsius"));
        assert!(url.contains("wind_speed_unit=ms"));
        assert!(url.contains("precipitation_unit=mm"));
    }

    #[tokio::test]
    async fn geocode_url_encoding() {
        let (mock, client) = client_with(Ok(geocode_json()));
        client.geocode("København").await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(!url.contains("København"));
        assert!(url.contains("K%C3%B8benhavn"));
    }

    #[tokio::test]
    async fn daily_array_length_mismatch_errors() {
        let json = r#"{
            "utc_offset_seconds": 0,
            "current": {
                "time": "2024-06-01T14:00",
                "temperature_2m": 15.3,
                "relative_humidity_2m": 65,
                "weather_code": 1,
                "wind_speed_10m": 3.5
            },
            "daily": {
                "time": ["2024-06-01", "2024-06-02"],
                "weather_code": [1],
                "temperature_2m_max": [18.0, 19.5],
                "temperature_2m_min": [12.0, 13.0],
                "sunrise": ["2024-06-01T05:30", "2024-06-02T05:29"],
                "sunset": ["2024-06-01T21:45", "2024-06-02T21:46"],
                "precipitation_sum": [0.0, 2.5]
            }
        }"#
        .into();
        let (_, client) = client_with(Ok(json));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::Parse { reason } if reason.contains("daily array lengths do not match"))
        );
    }

    #[tokio::test]
    async fn invalid_current_time_errors() {
        let json = r#"{
            "utc_offset_seconds": 0,
            "current": {
                "time": "not-a-time",
                "temperature_2m": 15.3,
                "relative_humidity_2m": 65,
                "weather_code": 1,
                "wind_speed_10m": 3.5
            },
            "daily": {
                "time": ["2024-06-01"],
                "weather_code": [1],
                "temperature_2m_max": [18.0],
                "temperature_2m_min": [12.0],
                "sunrise": ["2024-06-01T05:30"],
                "sunset": ["2024-06-01T21:45"],
                "precipitation_sum": [0.0]
            }
        }"#
        .into();
        let (_, client) = client_with(Ok(json));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse { reason } if reason.contains("invalid current.time")));
    }

    #[tokio::test]
    async fn invalid_utc_offset_errors() {
        let json = r#"{
            "utc_offset_seconds": 86401,
            "current": {
                "time": "2024-06-01T14:00",
                "temperature_2m": 15.3,
                "relative_humidity_2m": 65,
                "weather_code": 1,
                "wind_speed_10m": 3.5
            },
            "daily": {
                "time": ["2024-06-01"],
                "weather_code": [1],
                "temperature_2m_max": [18.0],
                "temperature_2m_min": [12.0],
                "sunrise": ["2024-06-01T05:30"],
                "sunset": ["2024-06-01T21:45"],
                "precipitation_sum": [0.0]
            }
        }"#
        .into();
        let (_, client) = client_with(Ok(json));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::Parse { reason } if reason.contains("invalid utc_offset_seconds"))
        );
    }

    #[tokio::test]
    async fn invalid_sunrise_format_errors() {
        let json = r#"{
            "utc_offset_seconds": 0,
            "daily": {
                "time": ["2024-06-01"],
                "weather_code": [1],
                "temperature_2m_max": [18.0],
                "temperature_2m_min": [12.0],
                "sunrise": ["bad-time"],
                "sunset": ["2024-06-01T21:45"],
                "precipitation_sum": [0.0]
            }
        }"#
        .into();
        let (_, client) = client_with(Ok(json));
        let loc = Location {
            name: None,
            latitude: 0.0,
            longitude: 0.0,
            country: None,
        };
        let err = client
            .fetch_forecast(&loc, 1, Units::Metric, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse { reason } if reason.contains("invalid time")));
    }
}

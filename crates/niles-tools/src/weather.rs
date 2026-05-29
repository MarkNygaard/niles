//! Weather tool — expose Open-Meteo to the LLM as `get_weather`.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_weather::{Location, OpenMeteoClient, Units};
use serde_json::{Value, json};
use std::sync::Arc;

fn map_weather_err<T>(r: std::result::Result<T, niles_weather::Error>) -> Result<T> {
    r.map_err(|e| Error::Weather(e.to_string()))
}

pub struct WeatherTool {
    client: Arc<OpenMeteoClient>,
    default_lat: f64,
    default_lon: f64,
    default_units: Units,
}

impl WeatherTool {
    pub fn new(
        client: Arc<OpenMeteoClient>,
        default_lat: f64,
        default_lon: f64,
        default_units: Units,
    ) -> Self {
        Self {
            client,
            default_lat,
            default_lon,
            default_units,
        }
    }
}

#[async_trait]
impl Tool for WeatherTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_weather".into(),
            description: "Fetch a weather forecast for a location. When no location is provided, \
                the forecast uses the home coordinates configured in niles."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "Place name to look up (e.g. 'Aarhus', 'Copenhagen'). \
                            Omit to use the home location."
                    },
                    "days": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 16,
                        "default": 1,
                        "description": "Number of forecast days (1–16)."
                    },
                    "include_current": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include current conditions in the response."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let location_query = args.get("location").and_then(|v| v.as_str());
        let days = args
            .get("days")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8)
            .unwrap_or(1);
        let include_current = args
            .get("include_current")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if days < 1 {
            return Err(Error::InvalidArgs {
                tool: "get_weather".into(),
                reason: "days must be >= 1".into(),
            });
        }
        let days = days.min(16);

        let location = if let Some(q) = location_query {
            map_weather_err(self.client.geocode(q).await)?
        } else {
            Location {
                name: None,
                latitude: self.default_lat,
                longitude: self.default_lon,
                country: None,
            }
        };

        let report = map_weather_err(
            self.client
                .fetch_forecast(&location, days, self.default_units, include_current)
                .await,
        )?;

        serde_json::to_value(&report).map_err(Error::Json)
    }
}

/// Register the weather tool onto an existing registry.
pub fn register_weather_tools(
    reg: &mut ToolRegistry,
    client: Arc<OpenMeteoClient>,
    lat: f64,
    lon: f64,
    units: Units,
) {
    reg.register(Box::new(WeatherTool::new(client, lat, lon, units)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use niles_weather::{Units, WeatherTransport};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_url: Arc<Mutex<Option<String>>>,
        responses: Arc<Mutex<Vec<std::result::Result<String, niles_weather::Error>>>>,
    }

    impl MockTransport {
        fn new(responses: Vec<std::result::Result<String, niles_weather::Error>>) -> Self {
            Self {
                last_url: Arc::new(Mutex::new(None)),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl WeatherTransport for MockTransport {
        async fn get(&self, url: &str) -> std::result::Result<String, niles_weather::Error> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }
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
                "time": ["2024-06-01"],
                "weather_code": [1],
                "temperature_2m_max": [18.0],
                "temperature_2m_min": [12.0],
                "sunrise": ["2024-06-01T05:30"],
                "sunset": ["2024-06-01T21:45"],
                "precipitation_sum": [0.0]
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

    fn tool_with(
        responses: Vec<std::result::Result<String, niles_weather::Error>>,
    ) -> (MockTransport, WeatherTool) {
        let mock = MockTransport::new(responses);
        let client = Arc::new(OpenMeteoClient::with_transport(
            Arc::new(mock.clone()),
            Default::default(),
        ));
        let tool = WeatherTool::new(client, 55.0, 10.0, Units::Metric);
        (mock, tool)
    }

    #[tokio::test]
    async fn default_location_no_geocode() {
        let (mock, tool) = tool_with(vec![Ok(forecast_json())]);
        let result = tool.execute(json!({})).await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("latitude=55"));
        assert!(url.contains("longitude=10"));
        assert!(!url.contains("geocoding-api"));
        assert!(result.get("location").is_some());
    }

    #[tokio::test]
    async fn named_location_geocodes_first() {
        let (mock, tool) = tool_with(vec![Ok(geocode_json()), Ok(forecast_json())]);
        let result = tool.execute(json!({"location": "Aarhus"})).await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("forecast"));
        assert!(result.get("location").is_some());
    }

    #[tokio::test]
    async fn days_zero_errors() {
        let (_mock, tool) = tool_with(vec![Ok(forecast_json())]);
        let err = tool.execute(json!({"days": 0})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "get_weather" && reason.contains("days must be >= 1"))
        );
    }

    #[tokio::test]
    async fn days_clamped_to_16() {
        let (mock, tool) = tool_with(vec![Ok(forecast_json())]);
        tool.execute(json!({"days": 50})).await.unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("forecast_days=16"));
    }

    #[tokio::test]
    async fn returned_json_has_expected_keys() {
        let (_mock, tool) = tool_with(vec![Ok(forecast_json())]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.get("location").is_some());
        assert!(result.get("units").is_some());
        assert!(result.get("current").is_some());
        assert!(result.get("daily").is_some());
    }
}

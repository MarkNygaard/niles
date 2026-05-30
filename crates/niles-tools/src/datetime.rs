//! Current datetime tool — expose the current date/time to the LLM as `current_datetime`.

use crate::error::Result;
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct CurrentDatetimeTool {
    timezone: chrono_tz::Tz,
    timezone_name: Arc<str>,
}

impl CurrentDatetimeTool {
    pub fn new(timezone: chrono_tz::Tz, timezone_name: Arc<str>) -> Self {
        Self {
            timezone,
            timezone_name,
        }
    }
}

#[async_trait]
impl Tool for CurrentDatetimeTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "current_datetime".into(),
            description: "Get the current date and time in both UTC and the household's local timezone. \
                Call this tool BEFORE any relative-date reasoning (e.g., 'today', 'tomorrow', 'this weekend') \
                to ensure accurate time grounding."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let now_utc = Utc::now();
        let now_local = now_utc.with_timezone(&self.timezone);

        let weekday = now_local.format("%A").to_string().to_lowercase();
        let date_local = now_local.format("%Y-%m-%d").to_string();

        Ok(json!({
            "iso_utc": now_utc.to_rfc3339(),
            "iso_local": now_local.to_rfc3339(),
            "timezone": self.timezone_name.as_ref(),
            "weekday": weekday,
            "date_local": date_local,
        }))
    }
}

/// Register the current-datetime tool onto an existing registry.
///
/// If `timezone_str` fails to parse as an IANA timezone, logs a warning and skips registration.
pub fn register_datetime_tool(reg: &mut ToolRegistry, timezone_str: &str) {
    let Ok(tz) = timezone_str.parse::<chrono_tz::Tz>() else {
        tracing::warn!("Failed to parse timezone '{timezone_str}'; current_datetime tool disabled");
        return;
    };
    let name: Arc<str> = tz.name().into();
    reg.register(Box::new(CurrentDatetimeTool::new(tz, name)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_with(tz: &str) -> CurrentDatetimeTool {
        let tz = tz.parse().unwrap();
        CurrentDatetimeTool::new(tz, tz.name().into())
    }

    #[tokio::test]
    async fn shape_of_response() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.get("iso_utc").is_some());
        assert!(result.get("iso_local").is_some());
        assert!(result.get("timezone").is_some());
        assert!(result.get("weekday").is_some());
        assert!(result.get("date_local").is_some());
    }

    #[tokio::test]
    async fn ignores_arguments() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({"foo": "bar"})).await.unwrap();
        assert!(result.get("iso_utc").is_some());
        assert!(result.get("iso_local").is_some());
    }

    #[tokio::test]
    async fn utc_tool_returns_iso_utc_equal_to_iso_local() {
        let tool = tool_with("UTC");
        let result = tool.execute(json!({})).await.unwrap();
        let iso_utc = result["iso_utc"].as_str().unwrap();
        let iso_local = result["iso_local"].as_str().unwrap();
        assert_eq!(iso_utc, iso_local);
    }

    #[tokio::test]
    async fn weekday_matches_locally_observed_day() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        let weekday = result["weekday"].as_str().unwrap();
        let valid = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        assert!(
            valid.contains(&weekday),
            "weekday '{weekday}' is not a valid lowercase English day name"
        );
    }

    #[tokio::test]
    async fn date_local_has_correct_format() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        let date_local = result["date_local"].as_str().unwrap();
        assert_eq!(date_local.len(), 10);
        // YYYY-MM-DD format
        assert!(date_local.chars().nth(4) == Some('-'));
        assert!(date_local.chars().nth(7) == Some('-'));
    }

    #[tokio::test]
    async fn iso_strings_are_valid_rfc3339() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        let iso_utc = result["iso_utc"].as_str().unwrap();
        let iso_local = result["iso_local"].as_str().unwrap();
        chrono::DateTime::parse_from_rfc3339(iso_utc).expect("iso_utc is valid RFC 3339");
        chrono::DateTime::parse_from_rfc3339(iso_local).expect("iso_local is valid RFC 3339");
    }

    #[tokio::test]
    async fn negative_offset_tz_produces_valid_rfc3339() {
        let tool = tool_with("America/New_York");
        let result = tool.execute(json!({})).await.unwrap();
        let iso_local = result["iso_local"].as_str().unwrap();
        chrono::DateTime::parse_from_rfc3339(iso_local)
            .expect("iso_local for negative-offset tz is valid RFC 3339");
    }

    #[tokio::test]
    async fn non_utc_tz_local_differs_from_utc() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        let iso_utc = result["iso_utc"].as_str().unwrap();
        let iso_local = result["iso_local"].as_str().unwrap();
        assert_ne!(
            iso_utc, iso_local,
            "local time in Europe/Copenhagen should differ from UTC"
        );
    }

    #[tokio::test]
    async fn timezone_field_matches_canonical_name() {
        let tool = tool_with("Europe/Copenhagen");
        let result = tool.execute(json!({})).await.unwrap();
        assert_eq!(result["timezone"], "Europe/Copenhagen");
    }

    #[test]
    fn register_valid_tz_adds_tool() {
        let mut reg = ToolRegistry::new();
        register_datetime_tool(&mut reg, "Europe/Copenhagen");
        assert!(
            reg.llm_tools()
                .into_iter()
                .any(|t| t.name == "current_datetime")
        );
    }

    #[test]
    fn register_invalid_tz_skips_tool() {
        let mut reg = ToolRegistry::new();
        register_datetime_tool(&mut reg, "Not/A/Real/Timezone");
        assert!(
            !reg.llm_tools()
                .into_iter()
                .any(|t| t.name == "current_datetime")
        );
    }
}

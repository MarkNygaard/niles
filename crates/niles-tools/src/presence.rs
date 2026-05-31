//! Presence tools — `get_presence` and `set_presence`.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use chrono::Utc;
use niles_presence::{Override, PresenceAggregator};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct GetPresenceTool {
    aggregator: Arc<PresenceAggregator>,
}

impl GetPresenceTool {
    pub fn new(aggregator: Arc<PresenceAggregator>) -> Self {
        Self { aggregator }
    }
}

#[async_trait]
impl Tool for GetPresenceTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_presence".into(),
            description: "Get the current home presence state. Returns whether anyone is home, \
                the manual override setting, and per-source readings."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let snap = self.aggregator.snapshot(Utc::now());
        serde_json::to_value(&snap).map_err(Error::Json)
    }
}

pub struct SetPresenceTool {
    aggregator: Arc<PresenceAggregator>,
}

impl SetPresenceTool {
    pub fn new(aggregator: Arc<PresenceAggregator>) -> Self {
        Self { aggregator }
    }
}

#[async_trait]
impl Tool for SetPresenceTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "set_presence".into(),
            description: "Override the home presence state. Use 'home' to force home, \
                'away' to force away, or 'auto' to let sensors decide again."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "state": {
                        "type": "string",
                        "enum": ["home", "away", "auto"],
                        "description": "Desired presence override state."
                    }
                },
                "required": ["state"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let state =
            args.get("state")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::InvalidArgs {
                    tool: "set_presence".into(),
                    reason: "missing required 'state' argument".into(),
                })?;

        let ov = match state {
            "home" => Override::ForceHome,
            "away" => Override::ForceAway,
            "auto" => Override::Auto,
            other => {
                return Err(Error::InvalidArgs {
                    tool: "set_presence".into(),
                    reason: format!("unknown state '{other}', expected home|away|auto"),
                });
            }
        };
        self.aggregator.set_override(ov);

        let snap = self.aggregator.snapshot(Utc::now());
        serde_json::to_value(&snap).map_err(Error::Json)
    }
}

/// Register the presence tools onto an existing registry.
pub fn register_presence_tools(reg: &mut ToolRegistry, aggregator: Arc<PresenceAggregator>) {
    reg.register(Box::new(GetPresenceTool::new(aggregator.clone())));
    reg.register(Box::new(SetPresenceTool::new(aggregator)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use niles_presence::{PresenceSignal, PresenceSnapshot};
    use serde_json::json;

    fn agg_with_home() -> Arc<PresenceAggregator> {
        let agg = Arc::new(PresenceAggregator::new(chrono::Duration::minutes(5)));
        agg.ingest(PresenceSignal {
            source: "test".into(),
            anyone_home: true,
            observed_at: Utc::now(),
        });
        agg
    }

    #[tokio::test]
    async fn get_presence_returns_snapshot_json() {
        let agg = agg_with_home();
        let tool = GetPresenceTool::new(agg);
        let result = tool.execute(json!({})).await.unwrap();
        assert_eq!(result.get("state").unwrap().as_str().unwrap(), "home");
        assert_eq!(result.get("override").unwrap().as_str().unwrap(), "auto");
        assert_eq!(result.get("sources").unwrap().as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn set_presence_away_forces_force_away() {
        let agg = agg_with_home();
        let tool = SetPresenceTool::new(agg.clone());
        let result = tool.execute(json!({"state": "away"})).await.unwrap();
        assert_eq!(result.get("state").unwrap().as_str().unwrap(), "away");
        let snap: PresenceSnapshot = serde_json::from_value(result).unwrap();
        assert_eq!(snap.r#override, Override::ForceAway);
    }

    #[tokio::test]
    async fn set_presence_home_forces_force_home() {
        let agg = Arc::new(PresenceAggregator::new(chrono::Duration::minutes(5)));
        agg.ingest(PresenceSignal {
            source: "test".into(),
            anyone_home: false,
            observed_at: Utc::now(),
        });
        let tool = SetPresenceTool::new(agg.clone());
        let result = tool.execute(json!({"state": "home"})).await.unwrap();
        assert_eq!(result.get("state").unwrap().as_str().unwrap(), "home");
        let snap: PresenceSnapshot = serde_json::from_value(result).unwrap();
        assert_eq!(snap.r#override, Override::ForceHome);
    }

    #[tokio::test]
    async fn set_presence_auto_resets_override() {
        let agg = agg_with_home();
        let tool = SetPresenceTool::new(agg.clone());
        tool.execute(json!({"state": "away"})).await.unwrap();
        let result = tool.execute(json!({"state": "auto"})).await.unwrap();
        assert_eq!(result.get("override").unwrap().as_str().unwrap(), "auto");
        assert_eq!(result.get("state").unwrap().as_str().unwrap(), "home"); // signal still home
    }

    #[tokio::test]
    async fn set_presence_unknown_state_errors_with_invalid_args() {
        let agg = agg_with_home();
        let tool = SetPresenceTool::new(agg);
        let err = tool
            .execute(json!({"state": "vacation"}))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool: t, .. } if t == "set_presence"));
    }

    #[tokio::test]
    async fn set_presence_missing_state_errors() {
        let agg = agg_with_home();
        let tool = SetPresenceTool::new(agg);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool: t, .. } if t == "set_presence"));
    }
}

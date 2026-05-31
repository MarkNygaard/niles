//! List-recent-notifications tool — let the LLM recall what was announced.

use crate::error::Result;
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_notifications::NotificationCenter;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_LIMIT: usize = 50;

pub struct ListRecentNotificationsTool {
    center: Arc<NotificationCenter>,
}

impl ListRecentNotificationsTool {
    pub fn new(center: Arc<NotificationCenter>) -> Self {
        Self { center }
    }
}

#[async_trait]
impl Tool for ListRecentNotificationsTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_recent_notifications".into(),
            description:
                "Return the most recent notifications that were announced (or attempted). \
                Use this when the user asks 'what did you say?' or 'what just happened?'."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of notifications to return (1–50). Defaults to 10."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(10)
            .clamp(1, MAX_LIMIT);

        let recent = self.center.recent(limit);
        let items: Vec<Value> = recent
            .into_iter()
            .map(|n| {
                json!({
                    "id": n.id,
                    "text": n.text,
                    "priority": match n.priority {
                        niles_notifications::Priority::Routine => "routine",
                        niles_notifications::Priority::Important => "important",
                        niles_notifications::Priority::Urgent => "urgent",
                    },
                    "room": n.room,
                    "outcome": match n.outcome {
                        niles_notifications::DeliveryOutcome::Delivered => "delivered",
                        niles_notifications::DeliveryOutcome::Suppressed => "suppressed",
                        niles_notifications::DeliveryOutcome::Failed => "failed",
                    },
                    "created_at": n.created_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(json!({ "notifications": items }))
    }
}

/// Register the list-recent-notifications tool onto an existing registry.
pub fn register_list_recent_notifications_tool(
    reg: &mut ToolRegistry,
    center: Arc<NotificationCenter>,
) {
    reg.register(Box::new(ListRecentNotificationsTool::new(center)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_center() -> Arc<NotificationCenter> {
        Arc::new(NotificationCenter::new(10))
    }

    #[tokio::test]
    async fn empty_list() {
        let center = make_center();
        let tool = ListRecentNotificationsTool::new(center);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[tokio::test]
    async fn lists_recent_items() {
        let center = make_center();
        center.deliver("first", None, niles_notifications::Priority::Routine);
        center.deliver("second", None, niles_notifications::Priority::Important);
        let tool = ListRecentNotificationsTool::new(center);
        let result = tool.execute(json!({"limit": 2})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["text"], "second");
        assert_eq!(arr[1]["text"], "first");
    }

    #[tokio::test]
    async fn clamps_limit_to_max() {
        let center = Arc::new(NotificationCenter::new(60));
        for i in 0..60 {
            center.deliver(
                format!("msg-{i}"),
                None,
                niles_notifications::Priority::Routine,
            );
        }
        let tool = ListRecentNotificationsTool::new(center);
        let result = tool.execute(json!({"limit": 100})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert_eq!(arr.len(), 50);
    }

    #[tokio::test]
    async fn defaults_limit_to_ten() {
        let center = make_center();
        for i in 0..20 {
            center.deliver(
                format!("msg-{i}"),
                None,
                niles_notifications::Priority::Routine,
            );
        }
        let tool = ListRecentNotificationsTool::new(center);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert_eq!(arr.len(), 10);
    }

    #[tokio::test]
    async fn zero_limit_clamps_to_one() {
        let center = make_center();
        center.deliver("first", None, niles_notifications::Priority::Routine);
        center.deliver("second", None, niles_notifications::Priority::Routine);
        let tool = ListRecentNotificationsTool::new(center);
        let result = tool.execute(json!({"limit": 0})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["text"], "second");
    }

    #[tokio::test]
    async fn negative_limit_defaults_to_ten() {
        let center = make_center();
        for i in 0..15 {
            center.deliver(
                format!("msg-{i}"),
                None,
                niles_notifications::Priority::Routine,
            );
        }
        let tool = ListRecentNotificationsTool::new(center);
        // serde_json parses -1 as i64, as_u64() returns None, so default=10, then clamp(1,50)
        let result = tool.execute(json!({"limit": -1})).await.unwrap();
        let arr = result["notifications"].as_array().unwrap();
        assert_eq!(arr.len(), 10); // defaults to 10 because as_u64() returns None
    }
}

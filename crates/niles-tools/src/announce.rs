//! Announce tool — allow the LLM to speak a notification.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_notifications::{NotificationCenter, Priority};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct AnnounceTool {
    center: Arc<NotificationCenter>,
}

impl AnnounceTool {
    pub fn new(center: Arc<NotificationCenter>) -> Self {
        Self { center }
    }
}

#[async_trait]
impl Tool for AnnounceTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "announce".into(),
            description:
                "Speak an unprompted notification to a specific room or the default satellite. \
                Use this when the system needs to proactively inform the user of something \
                (e.g., a timer finished, a device state changed, a reminder)."
                    .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The message to speak. Keep it concise and natural."
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["routine", "important", "urgent"],
                        "description": "Routine = informational, may be suppressed during quiet hours. \
                            Important = should be delivered unless quiet hours are active. \
                            Urgent = always delivered, bypasses quiet hours."
                    },
                    "room": {
                        "type": "string",
                        "description": "Optional target room name. If omitted, the notification is recorded but not spoken (no target satellite is known)."
                    }
                },
                "required": ["text", "priority"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::InvalidArgs {
                tool: "announce".into(),
                reason: "missing or non-string 'text'".into(),
            })?;
        let priority_str = args
            .get("priority")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::InvalidArgs {
                tool: "announce".into(),
                reason: "missing or non-string 'priority'".into(),
            })?;
        let room = args.get("room").and_then(|v| v.as_str()).map(String::from);

        let priority = match priority_str {
            "routine" => Priority::Routine,
            "important" => Priority::Important,
            "urgent" => Priority::Urgent,
            other => {
                return Err(Error::InvalidArgs {
                    tool: "announce".into(),
                    reason: format!(
                        "unknown priority '{other}', expected routine|important|urgent"
                    ),
                });
            }
        };

        let notification = self.center.deliver(text, room.clone(), priority);

        let (status, reason) = match notification.outcome {
            niles_notifications::DeliveryOutcome::Delivered => ("delivered", None),
            niles_notifications::DeliveryOutcome::Suppressed => {
                ("suppressed", Some("quiet hours active".to_string()))
            }
            niles_notifications::DeliveryOutcome::Failed => {
                ("failed", Some("delivery backend unavailable".to_string()))
            }
        };

        Ok(json!({
            "id": notification.id,
            "status": status,
            "room": room,
            "reason": reason,
        }))
    }
}

/// Register the announce tool onto an existing registry.
pub fn register_announce_tool(reg: &mut ToolRegistry, center: Arc<NotificationCenter>) {
    reg.register(Box::new(AnnounceTool::new(center)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_center() -> Arc<NotificationCenter> {
        Arc::new(NotificationCenter::new(10))
    }

    #[tokio::test]
    async fn announce_routine_records_notification() {
        let center = make_center();
        let tool = AnnounceTool::new(center);
        let result = tool
            .execute(json!({"text": "hello", "priority": "routine"}))
            .await
            .unwrap();
        // No delivery backend attached → Failed, but notification is recorded.
        assert_eq!(result["status"], "failed");
        assert!(result["id"].as_str().unwrap().contains('-'));
    }

    #[tokio::test]
    async fn announce_with_room() {
        let center = make_center();
        let tool = AnnounceTool::new(center);
        let result = tool
            .execute(json!({"text": "hello", "priority": "important", "room": "kitchen"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed"); // no delivery backend
        assert_eq!(result["room"], "kitchen");
    }

    #[tokio::test]
    async fn announce_invalid_priority_errors() {
        let center = make_center();
        let tool = AnnounceTool::new(center);
        let err = tool
            .execute(json!({"text": "hello", "priority": "loud"}))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("loud"), "{msg}");
    }

    #[tokio::test]
    async fn announce_missing_text_errors() {
        let center = make_center();
        let tool = AnnounceTool::new(center);
        let err = tool
            .execute(json!({"priority": "routine"}))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("text"), "{msg}");
    }
}

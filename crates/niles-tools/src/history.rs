//! The history tools: what was said, and how the devices were, at a
//! time gone by.
//!
//! Offered only when there is a history to read (a database). An empty
//! answer says when the history begins rather than coming back as `[]`,
//! which the model read as a lookup gone wrong and tried again until it
//! ran out of steps.

use crate::builtin::{parse_device_id, parse_opt_rfc3339, parse_required_rfc3339, resolve_device};
use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_core::{DeviceId, DeviceRegistry, RoomName};
use niles_history::{
    CommandHistory, CommandQuery, KEPT_DAYS, StateEntry, StateHistory, StateQuery,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Format a `StateEntry` with the `room/name` device id the rest of
/// the tool surface uses.
fn state_entry_value(entry: &StateEntry) -> Value {
    json!({
        "ts": entry.ts,
        "device_id": format!("{}/{}", entry.device_id.room(), entry.device_id.name()),
        "state": entry.state,
    })
}

/// A device id as the history keeps it: with the source of the device
/// the registry has by that name, or Zigbee2MQTT's for one it no longer
/// has, whose past is still kept.
fn history_id(registry: &DeviceRegistry, tool: &'static str, raw: &str) -> Result<DeviceId> {
    match resolve_device(registry, tool, raw) {
        Ok(device) => Ok(device.id),
        Err(Error::DeviceNotFound { .. }) => parse_device_id(tool, raw),
        Err(e) => Err(e),
    }
}

/// The answer when nothing matched: when the history begins, so the
/// model says so instead of asking again.
fn nothing_found(earliest: Option<DateTime<Utc>>) -> Value {
    let note = match earliest {
        Some(t) => format!(
            "Nothing was recorded for that. The history begins at {} and keeps {KEPT_DAYS} \
             days. Say so rather than looking again.",
            t.to_rfc3339()
        ),
        None => "Nothing has been recorded yet: the history starts with the next change. Say \
                 so rather than looking again."
            .into(),
    };
    json!({ "found": [], "note": note })
}

fn storage(tool: &'static str, e: niles_history::Error) -> Error {
    match e {
        niles_history::Error::InvalidDateRange { reason } => Error::InvalidArgs {
            tool: tool.into(),
            reason,
        },
        e => Error::Internal(format!("{tool}: {e}")),
    }
}

fn limit_arg(args: &Value) -> Option<usize> {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

// ---------- QueryCommandHistory ----------

pub struct QueryCommandHistory {
    history: Arc<dyn CommandHistory>,
}

impl QueryCommandHistory {
    pub fn new(history: Arc<dyn CommandHistory>) -> Self {
        Self { history }
    }
}

#[async_trait]
impl Tool for QueryCommandHistory {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "query_command_history".into(),
            description: "Query the user's recent voice command history. Use this to resolve anaphora ('turn it off again') or answer retrospective questions ('what did I do this morning'). Returns an array of command entries newest-first.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "since": {
                        "type": "string",
                        "description": "RFC 3339 timestamp for the lower bound (inclusive)."
                    },
                    "until": {
                        "type": "string",
                        "description": "RFC 3339 timestamp for the upper bound (inclusive)."
                    },
                    "room": {
                        "type": "string",
                        "description": "Filter to a specific origin room (canonical lower_snake name)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max entries to return (default 50, max 500).",
                        "minimum": 1,
                        "maximum": 500
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "query_command_history";
        let q = CommandQuery {
            since: parse_opt_rfc3339(TOOL, &args, "since")?,
            until: parse_opt_rfc3339(TOOL, &args, "until")?,
            room: args.get("room").and_then(Value::as_str).map(str::to_string),
            limit: limit_arg(&args),
        };
        let entries = self.history.query(&q).await.map_err(|e| storage(TOOL, e))?;
        if entries.is_empty() {
            let earliest = self
                .history
                .earliest()
                .await
                .map_err(|e| storage(TOOL, e))?;
            return Ok(nothing_found(earliest));
        }
        Ok(serde_json::to_value(entries)?)
    }
}

/// Register the command history tool onto an existing registry.
pub fn register_history_tools(reg: &mut ToolRegistry, history: Arc<dyn CommandHistory>) {
    reg.register(Box::new(QueryCommandHistory::new(history)));
}

// ---------- QueryDeviceStateHistory ----------

pub struct QueryDeviceStateHistory {
    history: Arc<dyn StateHistory>,
    registry: Arc<DeviceRegistry>,
}

impl QueryDeviceStateHistory {
    pub fn new(history: Arc<dyn StateHistory>, registry: Arc<DeviceRegistry>) -> Self {
        Self { history, registry }
    }
}

#[async_trait]
impl Tool for QueryDeviceStateHistory {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "query_device_state_history".into(),
            description: "Query the recent device state change history. Use this to answer retrospective questions like 'when did the kitchen light turn off'. Each entry is the device's whole state from that moment on. Returns an array of state entries newest-first.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "since": {
                        "type": "string",
                        "description": "RFC 3339 timestamp for the lower bound (inclusive)."
                    },
                    "until": {
                        "type": "string",
                        "description": "RFC 3339 timestamp for the upper bound (inclusive)."
                    },
                    "device_id": {
                        "type": "string",
                        "description": "Room-qualified device id, e.g. 'kitchen/ceiling_light'."
                    },
                    "room": {
                        "type": "string",
                        "description": "Filter to a specific room (canonical lower_snake name). Ignored if device_id is set."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max entries to return (default 200, max 2000).",
                        "minimum": 1,
                        "maximum": 2000
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "query_device_state_history";
        let q = StateQuery {
            since: parse_opt_rfc3339(TOOL, &args, "since")?,
            until: parse_opt_rfc3339(TOOL, &args, "until")?,
            device_id: match args.get("device_id").and_then(Value::as_str) {
                Some(raw) => Some(history_id(&self.registry, TOOL, raw)?),
                None => None,
            },
            room: args.get("room").and_then(Value::as_str).map(str::to_string),
            limit: limit_arg(&args),
        };
        let entries = self.history.query(&q).await.map_err(|e| storage(TOOL, e))?;
        if entries.is_empty() {
            let earliest = self
                .history
                .earliest()
                .await
                .map_err(|e| storage(TOOL, e))?;
            return Ok(nothing_found(earliest));
        }
        Ok(json!(
            entries.iter().map(state_entry_value).collect::<Vec<_>>()
        ))
    }
}

// ---------- DeviceStateSnapshotAt ----------

pub struct DeviceStateSnapshotAt {
    history: Arc<dyn StateHistory>,
    registry: Arc<DeviceRegistry>,
}

impl DeviceStateSnapshotAt {
    pub fn new(history: Arc<dyn StateHistory>, registry: Arc<DeviceRegistry>) -> Self {
        Self { history, registry }
    }

    fn ids(&self, args: &Value) -> Result<Vec<DeviceId>> {
        const TOOL: &str = "device_state_snapshot_at";
        if let Some(arr) = args.get("device_ids").and_then(Value::as_array) {
            if arr.is_empty() {
                return Err(Error::InvalidArgs {
                    tool: TOOL.into(),
                    reason: "device_ids must contain at least one device id".into(),
                });
            }
            return arr
                .iter()
                .map(|v| {
                    let raw = v.as_str().ok_or_else(|| Error::InvalidArgs {
                        tool: TOOL.into(),
                        reason: "device_ids must be an array of strings".into(),
                    })?;
                    history_id(&self.registry, TOOL, raw)
                })
                .collect();
        }
        let Some(raw) = args.get("room").and_then(Value::as_str) else {
            return Err(Error::InvalidArgs {
                tool: TOOL.into(),
                reason: "must specify one of device_ids or room".into(),
            });
        };
        let room = RoomName::parse(raw).map_err(|_| Error::RoomNotFound { name: raw.into() })?;
        Ok(self
            .registry
            .list_room(&room)
            .into_iter()
            .map(|d| d.id)
            .collect())
    }
}

#[async_trait]
impl Tool for DeviceStateSnapshotAt {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "device_state_snapshot_at".into(),
            description: "Reconstruct the state of one or more devices at a specific point in time: each device's whole state as it was then. Use this to answer 'how were things at 8pm yesterday' and to compose set_device calls to put a past scene back.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "at": {
                        "type": "string",
                        "description": "RFC 3339 timestamp for the point-in-time snapshot."
                    },
                    "device_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "List of room-qualified device ids, e.g. ['kitchen/ceiling_light', 'living_room/floor_lamp']."
                    },
                    "room": {
                        "type": "string",
                        "description": "Expand to all devices in this room. Ignored if device_ids is set."
                    }
                },
                "required": ["at"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "device_state_snapshot_at";
        let at = parse_required_rfc3339(TOOL, &args, "at")?;
        let ids = self.ids(&args)?;
        let entries = self
            .history
            .snapshot_at(at, &ids)
            .await
            .map_err(|e| storage(TOOL, e))?;
        if entries.is_empty() {
            let earliest = self
                .history
                .earliest()
                .await
                .map_err(|e| storage(TOOL, e))?;
            return Ok(nothing_found(earliest));
        }
        Ok(json!(
            entries.iter().map(state_entry_value).collect::<Vec<_>>()
        ))
    }
}

/// Register the state-history tools onto an existing registry.
pub fn register_state_history_tools(
    reg: &mut ToolRegistry,
    history: Arc<dyn StateHistory>,
    registry: Arc<DeviceRegistry>,
) {
    reg.register(Box::new(QueryDeviceStateHistory::new(
        history.clone(),
        registry.clone(),
    )));
    reg.register(Box::new(DeviceStateSnapshotAt::new(history, registry)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use niles_core::{Device, DeviceClass, DeviceState};
    use niles_history::{CommandEntry, MemoryCommandHistory, MemoryStateHistory};

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 30, hour, 0, 0).unwrap()
    }

    fn lit(brightness: u8) -> DeviceState {
        DeviceState {
            on: Some(true),
            brightness: Some(brightness),
            ..Default::default()
        }
    }

    fn registry() -> Arc<DeviceRegistry> {
        let registry = Arc::new(DeviceRegistry::new());
        for id in [
            "z2m:living_room/floor_lamp",
            "wled:living_room/strip",
            "z2m:kitchen/ceiling_light",
        ] {
            registry.upsert(Device::new(
                DeviceId::parse(id).unwrap(),
                DeviceState::default(),
                DeviceClass::Light,
            ));
        }
        registry
    }

    async fn kept(entries: &[(&str, DateTime<Utc>, u8)]) -> Arc<MemoryStateHistory> {
        let history = Arc::new(MemoryStateHistory::new());
        for (id, ts, brightness) in entries {
            history
                .record(&StateEntry {
                    ts: *ts,
                    device_id: DeviceId::parse(id).unwrap(),
                    state: lit(*brightness),
                })
                .await
                .unwrap();
        }
        history
    }

    #[tokio::test]
    async fn a_room_s_snapshot_is_each_device_as_it_was_whatever_its_source() {
        let history = kept(&[
            ("z2m:living_room/floor_lamp", at(19), 30),
            ("wled:living_room/strip", at(19), 50),
            ("z2m:living_room/floor_lamp", at(21), 100),
        ])
        .await;
        let tool = DeviceStateSnapshotAt::new(history, registry());
        let result = tool
            .execute(json!({ "at": at(20).to_rfc3339(), "room": "living_room" }))
            .await
            .unwrap();
        let then: Vec<_> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["device_id"].clone(), e["state"]["brightness"].clone()))
            .collect();
        assert_eq!(
            then,
            [
                (json!("living_room/strip"), json!(50)),
                (json!("living_room/floor_lamp"), json!(30)),
            ]
        );
    }

    #[tokio::test]
    async fn a_named_device_is_found_under_its_own_source() {
        let history = kept(&[("wled:living_room/strip", at(19), 50)]).await;
        let tool = DeviceStateSnapshotAt::new(history.clone(), registry());
        let result = tool
            .execute(json!({ "at": at(20).to_rfc3339(), "device_ids": ["living_room/strip"] }))
            .await
            .unwrap();
        assert_eq!(result[0]["state"]["brightness"], 50);

        let tool = QueryDeviceStateHistory::new(history, registry());
        let result = tool
            .execute(json!({ "device_id": "living_room/strip" }))
            .await
            .unwrap();
        assert_eq!(result.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_device_no_longer_there_still_has_its_past() {
        let history = kept(&[("z2m:hall/old_lamp", at(19), 40)]).await;
        let tool = DeviceStateSnapshotAt::new(history, registry());
        let result = tool
            .execute(json!({ "at": at(20).to_rfc3339(), "device_ids": ["hall/old_lamp"] }))
            .await
            .unwrap();
        assert_eq!(result[0]["device_id"], "hall/old_lamp");
    }

    #[tokio::test]
    async fn nothing_from_before_the_history_says_when_it_begins() {
        let history = kept(&[("z2m:living_room/floor_lamp", at(21), 30)]).await;
        let tool = DeviceStateSnapshotAt::new(history, registry());
        let result = tool
            .execute(json!({ "at": at(20).to_rfc3339(), "room": "living_room" }))
            .await
            .unwrap();
        assert_eq!(result["found"], json!([]));
        let note = result["note"].as_str().unwrap();
        assert!(note.contains(&at(21).to_rfc3339()), "{note}");
        assert!(note.contains("rather than looking again"), "{note}");
    }

    #[tokio::test]
    async fn nothing_kept_at_all_says_the_history_starts_with_the_next_change() {
        let tool = QueryDeviceStateHistory::new(Arc::new(MemoryStateHistory::new()), registry());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(
            result["note"]
                .as_str()
                .unwrap()
                .contains("starts with the next change")
        );
    }

    #[tokio::test]
    async fn a_query_is_newest_first() {
        let history = kept(&[
            ("z2m:kitchen/ceiling_light", at(19), 80),
            ("z2m:kitchen/ceiling_light", at(20), 40),
        ])
        .await;
        let tool = QueryDeviceStateHistory::new(history, registry());
        let result = tool
            .execute(json!({ "room": "kitchen", "since": at(18).to_rfc3339() }))
            .await
            .unwrap();
        assert_eq!(result[0]["state"]["brightness"], 40);
        assert_eq!(result[1]["state"]["brightness"], 80);
    }

    #[tokio::test]
    async fn bad_arguments_are_refused_by_name() {
        let history = Arc::new(MemoryStateHistory::new());
        let query = QueryDeviceStateHistory::new(history.clone(), registry());
        let err = query
            .execute(json!({ "since": "not-a-timestamp" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, .. } if tool == "query_device_state_history")
        );
        let err = query
            .execute(json!({ "since": at(21).to_rfc3339(), "until": at(20).to_rfc3339() }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { reason, .. } if reason.contains("since > until"))
        );

        let snapshot = DeviceStateSnapshotAt::new(history, registry());
        let err = snapshot
            .execute(json!({ "at": at(20).to_rfc3339() }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { reason, .. } if reason.contains("device_ids or room"))
        );
        let err = snapshot
            .execute(json!({ "at": at(20).to_rfc3339(), "device_ids": [] }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { reason, .. } if reason.contains("at least one"))
        );
    }

    #[tokio::test]
    async fn what_was_said_is_found_and_an_empty_answer_says_why() {
        let history = Arc::new(MemoryCommandHistory::new());
        let tool = QueryCommandHistory::new(history.clone());
        let none = tool.execute(json!({})).await.unwrap();
        assert!(
            none["note"]
                .as_str()
                .unwrap()
                .contains("Nothing has been recorded")
        );

        history
            .record(&CommandEntry {
                ts: at(20),
                peer: "192.168.69.188:58583".parse().unwrap(),
                origin_room: Some("living_room".into()),
                transcript: "movie time".into(),
                spoken_response: Some("Enjoy the film.".into()),
                speaker: Some("Mark".into()),
            })
            .await
            .unwrap();
        let found = tool
            .execute(json!({ "room": "living_room" }))
            .await
            .unwrap();
        assert_eq!(found[0]["transcript"], "movie time");
    }

    #[test]
    fn registering_offers_all_three() {
        let mut reg = ToolRegistry::new();
        register_state_history_tools(&mut reg, Arc::new(MemoryStateHistory::new()), registry());
        register_history_tools(&mut reg, Arc::new(MemoryCommandHistory::new()));
        let names: Vec<String> = reg.llm_tools().into_iter().map(|t| t.name).collect();
        for name in [
            "query_device_state_history",
            "device_state_snapshot_at",
            "query_command_history",
        ] {
            assert!(names.contains(&name.to_string()), "{name}");
        }
    }
}

//! Automation tools — `propose_automation`, `create_automation`,
//! `list_automations`, `pause_automation`, `remove_automation` and
//! `automation_runs`.
//!
//! A rule made by voice is read back before it is saved: the model
//! proposes it, says the sentence, and creates it only on a yes. A
//! misheard rule would otherwise go live in a house nobody is looking
//! at. Once saved it is on the Automations page like any other.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_automations::store::DEFAULT_SOURCE;
use niles_automations::{AutomationStore, Rule, RunLog, sentence};
use niles_config::{ActionDto, AutomationRuleDto, ConditionDto, TriggerDto, TriggerKind};
use niles_core::{DeviceId, DeviceRegistry, RoomName};
use serde_json::{Value, json};
use std::sync::Arc;

/// The rule as the model writes it — the same JSON the app sends.
fn rule_schema() -> Value {
    let device = json!({
        "type": "string",
        "description": "A device as room/name, e.g. office/door, office/go."
    });
    let duration = json!({
        "type": "object",
        "description": "How long, as hours/minutes/seconds.",
        "properties": {
            "hours": { "type": "integer", "minimum": 0 },
            "minutes": { "type": "integer", "minimum": 0 },
            "seconds": { "type": "integer", "minimum": 0 }
        }
    });
    let days = json!({
        "type": "array",
        "items": { "type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] }
    });
    let condition = json!({
        "type": "object",
        "description": "kind=time_of_day: after/before HH:MM (may wrap midnight), optional days. kind=sun: after and/or before sunrise|sunset, with after_offset_minutes/before_offset_minutes (after sunset + before sunrise = while it is dark). kind=device_is: a device on/off. kind=contact_is: a door open/closed. kind=numeric: a reading (measure temperature|humidity|battery|brightness) above and/or below a value. kind=presence_is: state home|away. kind=triggered_by: which of the rule's triggers (by id) started it — for a rule with several triggers doing different things.",
        "properties": {
            "kind": { "type": "string", "enum": ["time_of_day", "sun", "device_is", "contact_is", "numeric", "presence_is", "triggered_by"] },
            "after": { "type": "string" },
            "before": { "type": "string" },
            "after_offset_minutes": { "type": "integer" },
            "before_offset_minutes": { "type": "integer" },
            "days": days,
            "device": device,
            "on": { "type": "boolean" },
            "open": { "type": "boolean" },
            "measure": { "type": "string", "enum": ["temperature", "humidity", "battery", "brightness"] },
            "above": { "type": "number" },
            "below": { "type": "number" },
            "state": { "type": "string", "enum": ["home", "away"] },
            "triggers": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["kind"]
    });
    let action = json!({
        "type": "object",
        "description": "do=set_room: every light in a room (room, on, brightness 0-100, kelvin 2000-6500). do=set_device: one device. do=notify: say body in a room (a room with a satellite). do=delay: wait hours/minutes/seconds, then go on. do=if: conditions, then (actions), else (actions) — at most 3 deep.",
        "properties": {
            "do": { "type": "string", "enum": ["set_room", "set_device", "notify", "delay", "if"] },
            "room": { "type": "string" },
            "device": device,
            "on": { "type": "boolean" },
            "brightness": { "type": "integer", "minimum": 0, "maximum": 100 },
            "kelvin": { "type": "integer", "minimum": 2000, "maximum": 6500 },
            "body": { "type": "string" },
            "hours": { "type": "integer", "minimum": 0 },
            "minutes": { "type": "integer", "minimum": 0 },
            "seconds": { "type": "integer", "minimum": 0 },
            "conditions": { "type": "array", "items": condition },
            "then": { "type": "array", "items": { "type": "object" } },
            "else": { "type": "array", "items": { "type": "object" } }
        },
        "required": ["do"]
    });
    json!({
        "type": "object",
        "properties": {
            "description": {
                "type": "string",
                "description": "A short name for the rule, e.g. 'Office lights off when the garden door opens'."
            },
            "triggers": {
                "type": "array",
                "minItems": 1,
                "description": "What starts it; one or more. Give each an id (t1, t2…) when conditions need to tell them apart. type=contact: a door or window opens (open=true) or closes (false). type=device_state: a device turns on/off. type=numeric: a reading goes above/below a value. type=device_action: a button press. type=presence: someone comes home (state=home) or everyone leaves (away). type=time: at HH:MM, optional days. type=sun: sunrise|sunset with offset_minutes (negative: before). type=timer_fired. contact, device_state, numeric and presence take `for`: fire only once it has held that long ('open for 10 minutes').",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "type": { "type": "string", "enum": ["contact", "device_state", "numeric", "device_action", "presence", "time", "sun", "timer_fired"] },
                        "device": device,
                        "room": { "type": "string" },
                        "open": { "type": "boolean" },
                        "on": { "type": "boolean" },
                        "measure": { "type": "string", "enum": ["temperature", "humidity", "battery", "brightness"] },
                        "above": { "type": "number" },
                        "below": { "type": "number" },
                        "action": { "type": "string" },
                        "state": { "type": "string", "enum": ["home", "away"] },
                        "at": { "type": "string", "description": "HH:MM" },
                        "days": days,
                        "event": { "type": "string", "enum": ["sunrise", "sunset"] },
                        "offset_minutes": { "type": "integer" },
                        "name": { "type": "string" },
                        "for": duration
                    },
                    "required": ["type"]
                }
            },
            "conditions": {
                "type": "array",
                "description": "Optional; all must hold for the actions to run.",
                "items": condition
            },
            "actions": {
                "type": "array",
                "minItems": 1,
                "description": "What it does, in order.",
                "items": action
            }
        },
        "required": ["description", "triggers", "actions"]
    })
}

/// Read the model's rule. It is not trusted to leave out what the app
/// would: an `id` it made up is replaced, and `enabled` starts on.
fn read_rule(tool: &str, args: &Value) -> Result<AutomationRuleDto> {
    let mut body = args.clone();
    let Some(object) = body.as_object_mut() else {
        return Err(invalid(tool, "the rule must be an object".into()));
    };
    object.insert("id".into(), Value::String(String::new()));
    object.entry("enabled").or_insert(Value::Bool(true));
    object.entry("conditions").or_insert(json!([]));
    serde_json::from_value(body).map_err(|e| invalid(tool, format!("not a rule: {e}")))
}

/// What the rule names that this house does not have — a misheard
/// "office" as "offers". Checked here rather than in the store because
/// the app may well save a rule for a device not yet paired.
fn unknown(rule: &AutomationRuleDto, registry: &DeviceRegistry) -> Vec<String> {
    let mut devices: Vec<&str> = Vec::new();
    let mut rooms: Vec<&str> = Vec::new();
    for trigger in &rule.triggers {
        match &trigger.kind {
            TriggerKind::Contact { device, .. }
            | TriggerKind::DeviceAction { device, .. }
            | TriggerKind::Numeric { device, .. } => devices.push(device),
            TriggerKind::DeviceState { device, room, .. } => {
                devices.extend(device.as_deref());
                rooms.extend(room.as_deref());
            }
            _ => {}
        }
    }
    fn in_conditions<'a>(conditions: &'a [ConditionDto], devices: &mut Vec<&'a str>) {
        for condition in conditions {
            match condition {
                ConditionDto::DeviceIs { device, .. }
                | ConditionDto::ContactIs { device, .. }
                | ConditionDto::Numeric { device, .. } => devices.push(device),
                _ => {}
            }
        }
    }
    fn in_actions<'a>(
        actions: &'a [ActionDto],
        devices: &mut Vec<&'a str>,
        lit: &mut Vec<&'a str>,
    ) {
        for action in actions {
            match action {
                ActionDto::SetDevice { device, .. } => devices.push(device),
                ActionDto::SetRoom { room, .. } => lit.push(room),
                ActionDto::If {
                    conditions,
                    then,
                    otherwise,
                } => {
                    in_conditions(conditions, devices);
                    in_actions(then, devices, lit);
                    in_actions(otherwise, devices, lit);
                }
                _ => {}
            }
        }
    }
    in_conditions(&rule.conditions, &mut devices);
    in_actions(&rule.actions, &mut devices, &mut rooms);
    devices.sort_unstable();
    devices.dedup();
    rooms.sort_unstable();
    rooms.dedup();
    let mut missing = Vec::new();
    for device in devices {
        let full = if device.contains(':') {
            device.to_string()
        } else {
            format!("{DEFAULT_SOURCE}:{device}")
        };
        if DeviceId::parse(&full)
            .ok()
            .and_then(|id| registry.get(&id))
            .is_none()
        {
            missing.push(format!("no device called {device}"));
        }
    }
    for room in rooms {
        let lit = RoomName::parse(room)
            .map(|r| registry.list_room(&r).iter().any(|d| d.is_light()))
            .unwrap_or(false);
        if !lit {
            missing.push(format!("no lights in a room called {room}"));
        }
    }
    missing
}

/// Whether two rules start on any of the same kind of trigger for the
/// same thing — "you already have one for the garden door".
fn overlaps(a: &[TriggerDto], b: &[TriggerDto]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x.kind == y.kind))
}

pub struct ProposeAutomation {
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
}

#[async_trait]
impl Tool for ProposeAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "propose_automation".into(),
            description: "First step of making an automation — 'when the office door opens, \
                turn off the office lights', 'turn the lights off five minutes after the door \
                closes', 'tell me if the garden door has been open for ten minutes', 'turn on the \
                porch light at sunset'. Saves nothing. Returns the rule as a sentence: read it to \
                the person and ask whether to save it ('When the office door opens, I'll turn \
                off the office lights. Shall I?'). Call create_automation with the same rule \
                only after they say yes. If it returns problems, say them instead and ask what \
                they meant."
                .into(),
            parameters: rule_schema(),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let rule = read_rule("propose_automation", &args)?;
        let mut problems = unknown(&rule, &self.registry);
        if let Err(e) = Rule::from_dto(
            &AutomationRuleDto {
                id: "proposed".into(),
                ..rule.clone()
            },
            DEFAULT_SOURCE,
        ) {
            problems.push(e.to_string());
        }
        let similar: Vec<String> = self
            .store
            .list()
            .into_iter()
            .filter(|r| overlaps(&r.triggers, &rule.triggers))
            .map(|r| sentence(&r))
            .collect();
        Ok(json!({
            "sentence": sentence(&rule),
            "problems": problems,
            "existing_with_the_same_trigger": similar,
            "saved": false,
        }))
    }
}

pub struct CreateAutomation {
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
}

#[async_trait]
impl Tool for CreateAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "create_automation".into(),
            description: "Save an automation the person has just said yes to, after \
                propose_automation read it back. Never call this without that yes. Pass the \
                same rule propose_automation was given."
                .into(),
            parameters: rule_schema(),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let rule = read_rule("create_automation", &args)?;
        let problems = unknown(&rule, &self.registry);
        if !problems.is_empty() {
            return Err(invalid("create_automation", problems.join("; ")));
        }
        let made = self
            .store
            .create(rule)
            .map_err(|e| invalid("create_automation", e.to_string()))?;
        Ok(json!({ "saved": true, "id": made.id, "sentence": sentence(&made) }))
    }
}

pub struct ListAutomations {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for ListAutomations {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_automations".into(),
            description: "The household's automations: 'what automations do I have?'. Also \
                the way to find one's id before pausing, removing or asking about it. Say them \
                as their sentences."
                .into(),
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let rules: Vec<Value> = self
            .store
            .list()
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "name": r.description,
                    "sentence": sentence(&r),
                    "paused": !r.enabled,
                })
            })
            .collect();
        Ok(json!({ "automations": rules }))
    }
}

pub struct PauseAutomation {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for PauseAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "pause_automation".into(),
            description: "Pause an automation ('pause the garden door automation') or switch a \
                paused one back on (paused=false). Find its id with list_automations."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "paused": { "type": "boolean", "description": "true to pause, false to resume." }
                },
                "required": ["id", "paused"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("pause_automation", &args, "id")?;
        let paused = args.get("paused").and_then(Value::as_bool).unwrap_or(true);
        let rule = self
            .store
            .set_enabled(id, !paused)
            .map_err(|e| invalid("pause_automation", e.to_string()))?;
        Ok(json!({ "id": rule.id, "paused": !rule.enabled, "sentence": sentence(&rule) }))
    }
}

pub struct RemoveAutomation {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for RemoveAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "remove_automation".into(),
            description: "Delete an automation for good. Find it with list_automations, read \
                its sentence back and ask first ('Remove: when the office door opens, turn off \
                the office lights?'); call this only after a yes. To stop one for a while, \
                pause_automation instead."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("remove_automation", &args, "id")?;
        let gone = self
            .store
            .remove(id)
            .map_err(|e| invalid("remove_automation", e.to_string()))?;
        Ok(json!({ "removed": gone.id, "sentence": sentence(&gone) }))
    }
}

pub struct AutomationRuns {
    store: Arc<AutomationStore>,
    log: Arc<RunLog>,
}

#[async_trait]
impl Tool for AutomationRuns {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "automation_runs".into(),
            description: "What an automation did lately: 'why didn't the office light turn \
                off?', 'did the garden door rule run?'. Find its id with list_automations. Each \
                run says when, which trigger, and how it ended — done (with what it did), \
                stopped by a condition (and why), replaced by a newer run, or failed. Kept since \
                Niles last started."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("automation_runs", &args, "id")?;
        let rule = self
            .store
            .get(id)
            .ok_or_else(|| invalid("automation_runs", format!("there is no automation '{id}'")))?;
        let trigger = |run_trigger: &Option<String>| {
            run_trigger.as_ref().and_then(|t| {
                rule.triggers
                    .iter()
                    .position(|known| known.id.as_ref() == Some(t))
                    .map(|i| format!("trigger {}", i + 1))
            })
        };
        let runs: Vec<Value> = self
            .log
            .runs(id)
            .into_iter()
            .take(10)
            .map(|run| {
                json!({
                    "at": run.started_at.to_rfc3339(),
                    "trigger": trigger(&run.trigger),
                    "outcome": run.outcome,
                    "reason": run.reason,
                    "steps": run.steps,
                })
            })
            .collect();
        Ok(json!({ "sentence": sentence(&rule), "runs": runs }))
    }
}

fn required<'a>(tool: &str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(tool, format!("missing '{key}'")))
}

fn invalid(tool: &str, reason: String) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason,
    }
}

/// Register the automation tools over the store the app and the engine
/// share, and the log the engine writes its runs to.
pub fn register_automation_tools(
    reg: &mut ToolRegistry,
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
    log: Arc<RunLog>,
) {
    reg.register(Box::new(ProposeAutomation {
        store: store.clone(),
        registry: registry.clone(),
    }));
    reg.register(Box::new(CreateAutomation {
        store: store.clone(),
        registry,
    }));
    reg.register(Box::new(ListAutomations {
        store: store.clone(),
    }));
    reg.register(Box::new(PauseAutomation {
        store: store.clone(),
    }));
    reg.register(Box::new(RemoveAutomation {
        store: store.clone(),
    }));
    reg.register(Box::new(AutomationRuns { store, log }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{Device, DeviceClass, DeviceState};

    fn house() -> Arc<DeviceRegistry> {
        let registry = Arc::new(DeviceRegistry::new());
        let door = DeviceId::parse("z2m:office/door").unwrap();
        registry.upsert(Device::new(
            door,
            DeviceState {
                open: Some(false),
                ..Default::default()
            },
            DeviceClass::Contact,
        ));
        let go = DeviceId::parse("z2m:office/go").unwrap();
        registry.upsert(Device::new(
            go,
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        registry
    }

    fn garden_door(room: &str) -> Value {
        json!({
            "description": "Office lights off when the garden door opens",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
            "actions": [{ "do": "set_room", "room": room, "on": false }],
        })
    }

    #[tokio::test]
    async fn proposing_saves_nothing_and_says_the_rule() {
        let store = Arc::new(AutomationStore::new());
        let propose = ProposeAutomation {
            store: store.clone(),
            registry: house(),
        };
        let out = propose.execute(garden_door("office")).await.unwrap();
        assert_eq!(
            out["sentence"],
            "When the office door opens, turn off the office lights."
        );
        assert_eq!(out["problems"], json!([]));
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn a_misheard_room_is_a_problem_not_a_rule_even_inside_a_branch() {
        let store = Arc::new(AutomationStore::new());
        let propose = ProposeAutomation {
            store: store.clone(),
            registry: house(),
        };
        let out = propose.execute(garden_door("offers")).await.unwrap();
        assert_eq!(
            out["problems"],
            json!(["no lights in a room called offers"])
        );
        let branched = json!({
            "description": "x",
            "triggers": [{ "type": "contact", "device": "office/door", "open": false }],
            "actions": [{ "do": "if", "conditions": [], "then": [
                { "do": "delay", "minutes": 5 },
                { "do": "set_device", "device": "office/goo", "on": false }
            ] }],
        });
        let out = propose.execute(branched).await.unwrap();
        assert_eq!(out["problems"], json!(["no device called office/goo"]));
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
        };
        assert!(create.execute(garden_door("offers")).await.is_err());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn made_paused_and_removed_by_voice() {
        let store = Arc::new(AutomationStore::new());
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
        };
        let made = create.execute(garden_door("office")).await.unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        assert!(store.get(&id).unwrap().enabled);

        let listed = ListAutomations {
            store: store.clone(),
        }
        .execute(json!({}))
        .await
        .unwrap();
        assert_eq!(listed["automations"][0]["paused"], false);

        PauseAutomation {
            store: store.clone(),
        }
        .execute(json!({ "id": id, "paused": true }))
        .await
        .unwrap();
        assert!(!store.get(&id).unwrap().enabled);

        RemoveAutomation {
            store: store.clone(),
        }
        .execute(json!({ "id": id }))
        .await
        .unwrap();
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn the_model_does_not_choose_the_id() {
        let store = Arc::new(AutomationStore::new());
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
        };
        let mut rule = garden_door("office");
        rule["id"] = json!("whatever-it-likes");
        let made = create.execute(rule).await.unwrap();
        assert_eq!(made["id"], "office-lights-off-when-the-garden-door-opens");
    }

    #[tokio::test]
    async fn says_why_a_rule_did_not_run() {
        let store = Arc::new(AutomationStore::new());
        let log = Arc::new(RunLog::new());
        let made = CreateAutomation {
            store: store.clone(),
            registry: house(),
        }
        .execute(garden_door("office"))
        .await
        .unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        log.begin(
            &id,
            Some("t1"),
            chrono::Utc::now(),
            niles_automations::Outcome::Stopped,
            Some("it was not between 22:00 and 06:00".into()),
        );
        let out = AutomationRuns {
            store,
            log: log.clone(),
        }
        .execute(json!({ "id": id }))
        .await
        .unwrap();
        assert_eq!(out["runs"][0]["outcome"], "stopped");
        assert_eq!(out["runs"][0]["trigger"], "trigger 1");
        assert_eq!(
            out["runs"][0]["reason"],
            "it was not between 22:00 and 06:00"
        );
    }
}

//! Automation tools — `propose_automation`, `create_automation`,
//! `list_automations`, `pause_automation` and `remove_automation`.
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
use niles_automations::{AutomationStore, Rule, sentence};
use niles_config::{ActionDto, AutomationRuleDto, ConditionDto, TriggerDto};
use niles_core::{DeviceId, DeviceRegistry, RoomName};
use serde_json::{Value, json};
use std::sync::Arc;

/// The rule as the model writes it — the same JSON the app sends.
fn rule_schema() -> Value {
    let device = json!({
        "type": "string",
        "description": "A device as room/name, e.g. office/door, office/go."
    });
    json!({
        "type": "object",
        "properties": {
            "description": {
                "type": "string",
                "description": "A short name for the rule, e.g. 'Office lights off when the garden door opens'."
            },
            "trigger": {
                "type": "object",
                "description": "What starts it. type=contact: a door or window sensor opens (open=true) or closes (open=false). type=device_state: a device turns on or off. type=device_action: a button press (action e.g. on_press). type=presence: someone comes home (state=home) or everyone leaves (state=away). type=timer_fired.",
                "properties": {
                    "type": { "type": "string", "enum": ["contact", "device_state", "device_action", "presence", "timer_fired"] },
                    "device": device,
                    "room": { "type": "string" },
                    "open": { "type": "boolean" },
                    "on": { "type": "boolean" },
                    "action": { "type": "string" },
                    "state": { "type": "string", "enum": ["home", "away"] },
                    "name": { "type": "string" }
                },
                "required": ["type"]
            },
            "conditions": {
                "type": "array",
                "description": "Optional; all must hold. kind=time_of_day with after/before as HH:MM (may wrap midnight). kind=device_is: a device is on/off. kind=contact_is: a door is open/closed.",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["time_of_day", "device_is", "contact_is"] },
                        "after": { "type": "string" },
                        "before": { "type": "string" },
                        "device": device,
                        "on": { "type": "boolean" },
                        "open": { "type": "boolean" }
                    },
                    "required": ["kind"]
                }
            },
            "actions": {
                "type": "array",
                "minItems": 1,
                "description": "What it does, in order. do=set_room: every light in a room (room, on, brightness 0-100, kelvin 2000-6500). do=set_device: one device. do=notify: say body in a room.",
                "items": {
                    "type": "object",
                    "properties": {
                        "do": { "type": "string", "enum": ["set_room", "set_device", "notify"] },
                        "room": { "type": "string" },
                        "device": device,
                        "on": { "type": "boolean" },
                        "brightness": { "type": "integer", "minimum": 0, "maximum": 100 },
                        "kelvin": { "type": "integer", "minimum": 2000, "maximum": 6500 },
                        "body": { "type": "string" }
                    },
                    "required": ["do"]
                }
            }
        },
        "required": ["description", "trigger", "actions"]
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
    match &rule.trigger {
        TriggerDto::Contact { device, .. } | TriggerDto::DeviceAction { device, .. } => {
            devices.push(device)
        }
        TriggerDto::DeviceState { device, room, .. } => {
            devices.extend(device.as_deref());
            rooms.extend(room.as_deref());
        }
        _ => {}
    }
    for condition in &rule.conditions {
        match condition {
            ConditionDto::DeviceIs { device, .. } | ConditionDto::ContactIs { device, .. } => {
                devices.push(device)
            }
            ConditionDto::TimeOfDay { .. } => {}
        }
    }
    for action in &rule.actions {
        match action {
            ActionDto::SetDevice { device, .. } => devices.push(device),
            ActionDto::SetRoom { room, .. } => rooms.push(room),
            ActionDto::Notify { .. } => {}
        }
    }
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
                turn off the office lights', 'turn on the hallway light when we come home'. \
                Saves nothing. Returns the rule as a sentence: read it to the person and ask \
                whether to save it ('When the office door opens, I'll turn off the office \
                lights. Shall I?'). Call create_automation with the same rule only after they \
                say yes. If it returns problems, say them instead and ask what they meant."
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
            .filter(|r| r.trigger == rule.trigger)
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
                the way to find one's id before pausing or removing it. Say them as their \
                sentences."
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
/// share.
pub fn register_automation_tools(
    reg: &mut ToolRegistry,
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
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
    reg.register(Box::new(RemoveAutomation { store }));
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
            "trigger": { "type": "contact", "device": "office/door", "open": true },
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
    async fn a_misheard_room_is_a_problem_not_a_rule() {
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
}

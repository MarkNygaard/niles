//! Built-in LLM tools: device read/write + room/global listing.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::skill::register_skill_tools;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_capabilities::CapabilityLoader;
use niles_core::{DeviceClass, DeviceId, DeviceRegistry, DeviceState, RoomName};
use niles_history::{CommandQuery, CommandReader, StateQuery, StateReader};
use niles_memory::{MemoryStore, Target as MemoryTarget};
use niles_mqtt::{MqttPublisher, format_set_command, is_actionable};
use niles_scheduler::{TimerState, TimerStore, canonicalize_name};
use serde_json::{Value, json};
use std::sync::Arc;

/// Parse a `<room>/<name>` arg into a `DeviceId` by prepending `z2m:`.
fn parse_device_id(tool: &'static str, raw: &str) -> Result<DeviceId> {
    DeviceId::parse(&format!("z2m:{raw}")).map_err(|e| Error::InvalidArgs {
        tool: tool.into(),
        reason: format!("invalid device_id '{raw}': {e}"),
    })
}

fn required_str<'a>(tool: &'static str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.into(),
            reason: format!("missing required field '{key}'"),
        })
}

/// Parse an optional RFC 3339 timestamp argument into a UTC `DateTime`.
fn parse_opt_rfc3339(tool: &'static str, args: &Value, key: &str) -> Result<Option<DateTime<Utc>>> {
    match args.get(key).and_then(|v| v.as_str()) {
        Some(s) => DateTime::parse_from_rfc3339(s)
            .map_err(|e| Error::InvalidArgs {
                tool: tool.into(),
                reason: format!("invalid '{key}': {e}"),
            })
            .map(|dt| Some(dt.with_timezone(&Utc))),
        None => Ok(None),
    }
}

/// Parse a required RFC 3339 timestamp argument into a UTC `DateTime`.
fn parse_required_rfc3339(tool: &'static str, args: &Value, key: &str) -> Result<DateTime<Utc>> {
    let raw = required_str(tool, args, key)?;
    DateTime::parse_from_rfc3339(raw)
        .map_err(|e| Error::InvalidArgs {
            tool: tool.into(),
            reason: format!("invalid '{key}': {e}"),
        })
        .map(|dt| dt.with_timezone(&Utc))
}

fn device_summary(device: &niles_core::Device) -> Value {
    json!({
        "id": format!("{}/{}", device.id.room(), device.id.name()),
        "on": device.state.on,
        "brightness": device.state.brightness,
        "color_temp_kelvin": device.state.color_temp_kelvin,
    })
}

fn device_full(device: &niles_core::Device) -> Value {
    let mut v = device_summary(device);
    v["temperature_celsius"] = json!(device.state.temperature_celsius);
    v["humidity_percent"] = json!(device.state.humidity_percent);
    v["battery_percent"] = json!(device.state.battery_percent);
    v
}

/// Format a `StateEntry` as a JSON value using the same `room/name`
/// device-id convention that the rest of the tool surface uses.
fn state_entry_value(entry: &niles_history::StateEntry) -> Value {
    json!({
        "ts": entry.ts,
        "device_id": format!("{}/{}", entry.device_id.room(), entry.device_id.name()),
        "state": entry.state,
    })
}

/// Extract and validate `DeviceState` from `set_device` arguments.
/// Returns `InvalidArgs` if no state fields are provided or if values
/// are out of range.
pub(crate) fn extract_set_state(args: &Value) -> Result<DeviceState> {
    let on = args.get("on").and_then(|v| v.as_bool());

    let brightness = match args.get("brightness") {
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| Error::InvalidArgs {
                tool: "set_device".into(),
                reason: "brightness must be an integer".into(),
            })?;
            if n > 100 {
                return Err(Error::InvalidArgs {
                    tool: "set_device".into(),
                    reason: format!("brightness {n} exceeds maximum 100"),
                });
            }
            Some(n as u8)
        }
        None => None,
    };

    let color_temp_kelvin = match args.get("color_temp_kelvin") {
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| Error::InvalidArgs {
                tool: "set_device".into(),
                reason: "color_temp_kelvin must be an integer".into(),
            })?;
            if !(1000..=10000).contains(&n) {
                return Err(Error::InvalidArgs {
                    tool: "set_device".into(),
                    reason: format!("color_temp_kelvin {n} outside valid range 1000..=10000"),
                });
            }
            Some(n as u16)
        }
        None => None,
    };

    let state = DeviceState {
        on,
        brightness,
        color_temp_kelvin,
        ..Default::default()
    };

    if !is_actionable(&state) {
        return Err(Error::InvalidArgs {
            tool: "set_device".into(),
            reason: "must specify at least one of on, brightness, color_temp_kelvin".into(),
        });
    }

    Ok(state)
}

// ---------- explain_device formatters ----------

fn explain_device(device: &niles_core::Device) -> String {
    let id = format!("{}/{}", device.id.room(), device.id.name());
    match device.class {
        DeviceClass::Light => explain_light(&id, &device.state),
        DeviceClass::Switch => explain_switch(&id, &device.state),
        DeviceClass::Sensor => explain_sensor(&id, &device.state),
        // DeviceClass is #[non_exhaustive]; Unknown and future variants fall back to generic.
        _ => explain_unknown(&id, &device.state),
    }
}

fn explain_light(id: &str, state: &DeviceState) -> String {
    match state.on {
        Some(true) => {
            // Build detail clauses into a Vec so a missing brightness
            // doesn't leave a hanging comma before a kelvin-only suffix.
            let mut detail = Vec::new();
            if let Some(b) = state.brightness {
                detail.push(format!("{b}% brightness"));
            }
            if let Some(k) = state.color_temp_kelvin {
                detail.push(format!("color temperature {k}K"));
            }
            if detail.is_empty() {
                format!("{id} is on")
            } else {
                format!("{id} is on at {}", detail.join(", "))
            }
        }
        Some(false) => {
            // When the light is off we ignore brightness / kelvin.
            format!("{id} is off")
        }
        None => {
            // Without on/off we can't say anything meaningful; any
            // partial fields are still "not reported yet".
            format!("{id} is a light but its state hasn't been reported yet")
        }
    }
}

fn explain_switch(id: &str, state: &DeviceState) -> String {
    // No vendor/model metadata in the registry yet, so describe the device
    // class generically rather than guessing a brand.
    if let Some(pct) = state.battery_percent {
        format!("{id} is a button device; battery {pct}%")
    } else {
        format!("{id} is a button device; no battery report yet")
    }
}

fn explain_sensor(id: &str, state: &DeviceState) -> String {
    let mut parts = Vec::new();
    if let Some(t) = state.temperature_celsius {
        parts.push(format!("temperature {t:.1}°C"));
    }
    if let Some(h) = state.humidity_percent {
        parts.push(format!("humidity {h:.0}%"));
    }
    if let Some(b) = state.battery_percent {
        parts.push(format!("battery {b}%"));
    }
    if parts.is_empty() {
        format!("{id} is a sensor but no readings have been reported yet")
    } else {
        format!("{id} is a sensor; {}", parts.join(", "))
    }
}

fn explain_unknown(id: &str, state: &DeviceState) -> String {
    let mut parts = Vec::new();
    if let Some(v) = state.on {
        parts.push(format!("on: {v}"));
    }
    if let Some(v) = state.brightness {
        parts.push(format!("brightness: {v}"));
    }
    if let Some(v) = state.color_temp_kelvin {
        parts.push(format!("color_temp_kelvin: {v}"));
    }
    if let Some(v) = state.temperature_celsius {
        parts.push(format!("temperature_celsius: {v:.1}"));
    }
    if let Some(v) = state.humidity_percent {
        parts.push(format!("humidity_percent: {v:.0}"));
    }
    if let Some(v) = state.battery_percent {
        parts.push(format!("battery_percent: {v}"));
    }
    if parts.is_empty() {
        format!("{id} is an unclassified device with no state reported")
    } else {
        format!("{id} is an unclassified device; {}", parts.join(", "))
    }
}

// ---------- GetDeviceState ----------

pub struct GetDeviceState {
    registry: Arc<DeviceRegistry>,
}

impl GetDeviceState {
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for GetDeviceState {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_device_state".into(),
            description: "Return current state of a single device.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": { "type": "string", "description": "Room-qualified id, e.g. 'kitchen/ceiling_light'." }
                },
                "required": ["device_id"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let raw = required_str("get_device_state", &args, "device_id")?;
        let id = parse_device_id("get_device_state", raw)?;
        let device = self
            .registry
            .get(&id)
            .ok_or_else(|| Error::DeviceNotFound { id: raw.into() })?;
        Ok(device_full(&device))
    }
}

// ---------- ExplainDeviceState ----------

pub struct ExplainDeviceState {
    registry: Arc<DeviceRegistry>,
}

impl ExplainDeviceState {
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ExplainDeviceState {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "explain_device_state".into(),
            description: "Return a human-readable one-sentence description of a single device's current state. Use this when responding to the user about a device, not get_device_state (which returns raw JSON).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": { "type": "string", "description": "Room-qualified id, e.g. 'kitchen/ceiling_light'." }
                },
                "required": ["device_id"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let raw = required_str("explain_device_state", &args, "device_id")?;
        let id = parse_device_id("explain_device_state", raw)?;
        let device = self
            .registry
            .get(&id)
            .ok_or_else(|| Error::DeviceNotFound { id: raw.into() })?;
        Ok(json!({ "explanation": explain_device(&device) }))
    }
}

// ---------- ListDevicesInRoom ----------

pub struct ListDevicesInRoom {
    registry: Arc<DeviceRegistry>,
}

impl ListDevicesInRoom {
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListDevicesInRoom {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_devices_in_room".into(),
            description: "List devices in a single room. Returns [] for an unknown or empty room."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "Canonical room name, lower_snake, e.g. 'living_room'." }
                },
                "required": ["room"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let raw = required_str("list_devices_in_room", &args, "room")?;
        let room = RoomName::parse(raw).map_err(|_| Error::RoomNotFound { name: raw.into() })?;
        let list: Vec<Value> = self
            .registry
            .list_room(&room)
            .iter()
            .map(device_summary)
            .collect();
        Ok(Value::Array(list))
    }
}

// ---------- ListAllDevices ----------

pub struct ListAllDevices {
    registry: Arc<DeviceRegistry>,
}

impl ListAllDevices {
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListAllDevices {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_all_devices".into(),
            description: "List every registered device.".into(),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let list: Vec<Value> = self
            .registry
            .list_all()
            .iter()
            .map(device_summary)
            .collect();
        Ok(Value::Array(list))
    }
}

#[async_trait]
pub trait Publisher: Send + Sync {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> niles_mqtt::Result<()>;
}

#[async_trait]
impl Publisher for MqttPublisher {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> niles_mqtt::Result<()> {
        self.publish(topic, payload).await
    }
}

// ---------- SetDevice ----------

pub struct SetDevice<P: Publisher = MqttPublisher> {
    registry: Arc<DeviceRegistry>,
    publisher: P,
    z2m_prefix: Arc<String>,
}

impl<P: Publisher> SetDevice<P> {
    pub fn new(registry: Arc<DeviceRegistry>, publisher: P, z2m_prefix: Arc<String>) -> Self {
        Self {
            registry,
            publisher,
            z2m_prefix,
        }
    }
}

#[async_trait]
impl<P: Publisher> Tool for SetDevice<P> {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "set_device".into(),
            description: "Set the state of a device. At least one of on/brightness/color_temp_kelvin is required.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": { "type": "string", "description": "Room-qualified id, e.g. 'kitchen/ceiling_light'." },
                    "on": { "type": "boolean" },
                    "brightness": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "color_temp_kelvin": { "type": "integer", "minimum": 1000, "maximum": 10000 }
                },
                "required": ["device_id"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let raw = required_str("set_device", &args, "device_id")?;
        let id = parse_device_id("set_device", raw)?;

        let device = self
            .registry
            .get(&id)
            .ok_or_else(|| Error::DeviceNotFound { id: raw.into() })?;

        if !device.is_light() {
            return Err(Error::WrongDeviceClass {
                id: raw.into(),
                class: device.class,
            });
        }

        let target = extract_set_state(&args)?;
        let (topic, payload) = format_set_command(&self.z2m_prefix, &id, &target);
        tracing::debug!("publishing {topic} {payload}");
        self.publisher.publish(&topic, payload.into_bytes()).await?;
        Ok(json!({ "ok": true, "topic": topic }))
    }
}

// ---------- LookUpCapability ----------

pub struct LookUpCapability {
    loader: Arc<CapabilityLoader>,
}

impl LookUpCapability {
    pub fn new(loader: Arc<CapabilityLoader>) -> Self {
        Self { loader }
    }
}

#[async_trait]
impl Tool for LookUpCapability {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "look_up_capability".into(),
            description: "Fetch a capability's markdown body and metadata by name. On a miss, returns available names so the LLM can self-correct.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Capability name to look up." }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("look_up_capability", &args, "name")?;
        if let Some(cap) = self.loader.get(name) {
            Ok(json!({
                "found": true,
                "name": cap.metadata.name,
                "description": cap.metadata.description,
                "version": cap.metadata.version,
                "prerequisites": cap.metadata.prerequisites,
                "body": cap.body
            }))
        } else {
            Ok(json!({
                "found": false,
                "name": name,
                "available": self.loader.names()
            }))
        }
    }
}

// ---------- MemoryTool ----------

pub struct MemoryTool {
    store: Arc<MemoryStore>,
}

impl MemoryTool {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "memory".into(),
            description: "Manage persistent memory. Actions: 'add' requires 'content'; 'replace' requires 'old_text' and 'content'; 'remove' requires 'old_text'; 'view' needs only 'target'. Targets are 'user' (USER.md) or 'agent' (MEMORY.md).".into(),
            parameters: json!({
                "type": "object",
                "required": ["action", "target"],
                "properties": {
                    "action": { "type": "string", "enum": ["add", "replace", "remove", "view"], "description": "Action to perform." },
                    "target": { "type": "string", "enum": ["user", "agent"], "description": "'user' for household facts (USER.md) or 'agent' for learnings (MEMORY.md)." },
                    "content": { "type": "string", "description": "Text to add or replace with. Required for 'add' and 'replace'." },
                    "old_text": { "type": "string", "description": "Snippet to match for 'replace' or 'remove'. Must uniquely identify one entry." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = required_str("memory", &args, "action")?;
        let target_str = required_str("memory", &args, "target")?;
        let target = match target_str {
            "user" => MemoryTarget::User,
            "agent" => MemoryTarget::Memory,
            other => {
                return Err(Error::InvalidArgs {
                    tool: "memory".into(),
                    reason: format!("target must be 'user' or 'agent', got '{other}'"),
                });
            }
        };

        match action {
            "view" => {
                let entries = self
                    .store
                    .load(target)
                    .map_err(|e| Error::Memory(format!("{e}")))?;
                let lines: Vec<String> = entries.into_iter().map(|e| e.text).collect();
                Ok(json!({ "entries": lines }))
            }
            "add" => {
                let content = required_str("memory", &args, "content")?;
                self.store
                    .add(target, content)
                    .map_err(|e| Error::Memory(format!("{e}")))?;
                Ok(json!({ "ok": true, "action": "add", "target": target_str }))
            }
            "replace" => {
                let old_text = required_str("memory", &args, "old_text")?;
                let content = required_str("memory", &args, "content")?;
                self.store
                    .replace(target, old_text, content)
                    .map_err(|e| Error::Memory(format!("{e}")))?;
                Ok(json!({ "ok": true, "action": "replace", "target": target_str }))
            }
            "remove" => {
                let old_text = required_str("memory", &args, "old_text")?;
                self.store
                    .remove(target, old_text)
                    .map_err(|e| Error::Memory(format!("{e}")))?;
                Ok(json!({ "ok": true, "action": "remove", "target": target_str }))
            }
            other => Err(Error::InvalidArgs {
                tool: "memory".into(),
                reason: format!("action must be one of add/replace/remove/view, got '{other}'"),
            }),
        }
    }
}

/// Register the memory tool onto an existing registry.
pub fn register_memory_tools(reg: &mut ToolRegistry, store: Arc<MemoryStore>) {
    reg.register(Box::new(MemoryTool::new(store)));
}

fn timer_state_str(state: TimerState) -> &'static str {
    match state {
        TimerState::Pending => "pending",
        TimerState::Ringing => "ringing",
    }
}

// ---------- ListTimers ----------

pub struct ListTimers {
    timers: Arc<TimerStore>,
}

impl ListTimers {
    pub fn new(timers: Arc<TimerStore>) -> Self {
        Self { timers }
    }
}

#[async_trait]
impl Tool for ListTimers {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_timers".into(),
            description: "Return all active timers (pending or ringing). Each entry includes id, name, duration, state, and remaining seconds.".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let now = chrono::Utc::now();
        let entries: Vec<Value> = self
            .timers
            .list()
            .into_iter()
            .map(|e| {
                let remaining_s = (e.expires_at - now).num_seconds().max(0);
                json!({
                    "id": e.id.0,
                    "name": e.name,
                    "duration_seconds": e.duration.as_secs(),
                    "state": timer_state_str(e.state),
                    "remaining_seconds": remaining_s,
                })
            })
            .collect();
        Ok(json!({ "timers": entries }))
    }
}

// ---------- CancelTimer ----------

pub struct CancelTimer {
    timers: Arc<TimerStore>,
}

impl CancelTimer {
    pub fn new(timers: Arc<TimerStore>) -> Self {
        Self { timers }
    }
}

#[async_trait]
impl Tool for CancelTimer {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "cancel_timer".into(),
            description: "Cancel timer(s) by name. Matching is case-insensitive and whitespace-normalized (e.g. ' Pasta ' matches 'pasta'). Returns the count of timers cancelled; multiple timers may share a name and all matches are removed. 0 means no match.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Timer name, e.g. 'pasta'." }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("cancel_timer", &args, "name")?;
        let canonical = canonicalize_name(name);
        let count = self.timers.cancel_by_name(&canonical);
        Ok(json!({ "cancelled": count, "name": canonical }))
    }
}

// ---------- GetTimerRemaining ----------

pub struct GetTimerRemaining {
    timers: Arc<TimerStore>,
}

impl GetTimerRemaining {
    pub fn new(timers: Arc<TimerStore>) -> Self {
        Self { timers }
    }
}

#[async_trait]
impl Tool for GetTimerRemaining {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_timer_remaining".into(),
            description: "Return remaining seconds for a timer by name. Matching is case-insensitive and whitespace-normalized. If multiple timers share the name, the one expiring soonest is returned. Returns null in 'remaining_seconds' if no timer with that name is active.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Timer name, e.g. 'pasta'." }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = required_str("get_timer_remaining", &args, "name")?;
        let canonical = canonicalize_name(name);
        let now = chrono::Utc::now();
        let found = self
            .timers
            .list()
            .into_iter()
            .find(|e| e.name.as_ref() == Some(&canonical));
        match found {
            Some(e) => {
                let remaining_s = (e.expires_at - now).num_seconds().max(0);
                Ok(json!({
                    "found": true,
                    "name": e.name,
                    "remaining_seconds": remaining_s,
                    "state": timer_state_str(e.state),
                }))
            }
            None => Ok(json!({
                "found": false,
                "name": canonical,
                "remaining_seconds": null,
            })),
        }
    }
}

/// Register the timer tools onto an existing registry. Separate from
/// `default_registry` because the timer store comes from
/// `niles-scheduler` at runtime, not from the LLM tools layer.
pub fn register_timer_tools(reg: &mut ToolRegistry, timers: Arc<TimerStore>) {
    reg.register(Box::new(ListTimers::new(timers.clone())));
    reg.register(Box::new(CancelTimer::new(timers.clone())));
    reg.register(Box::new(GetTimerRemaining::new(timers)));
}

// ---------- QueryCommandHistory ----------

pub struct QueryCommandHistory {
    reader: Arc<CommandReader>,
}

impl QueryCommandHistory {
    pub fn new(reader: Arc<CommandReader>) -> Self {
        Self { reader }
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
        let since = parse_opt_rfc3339("query_command_history", &args, "since")?;
        let until = parse_opt_rfc3339("query_command_history", &args, "until")?;
        let room = args
            .get("room")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let q = CommandQuery {
            since,
            until,
            room,
            limit,
        };

        let entries = self
            .reader
            .query(&q)
            .map_err(|e| Error::Internal(format!("query failed: {e}")))?;
        Ok(serde_json::to_value(entries)?)
    }
}

/// Register the history query tool onto an existing registry.
pub fn register_history_tools(reg: &mut ToolRegistry, reader: Arc<CommandReader>) {
    reg.register(Box::new(QueryCommandHistory::new(reader)));
}

// ---------- QueryDeviceStateHistory ----------

pub struct QueryDeviceStateHistory {
    reader: Arc<StateReader>,
}

impl QueryDeviceStateHistory {
    pub fn new(reader: Arc<StateReader>) -> Self {
        Self { reader }
    }
}

#[async_trait]
impl Tool for QueryDeviceStateHistory {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "query_device_state_history".into(),
            description: "Query the recent device state change history. Use this to answer retrospective questions like 'what was the light level at 8pm yesterday' or 'when did the kitchen light turn off'. Returns an array of state entries newest-first.".into(),
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
        let since = parse_opt_rfc3339("query_device_state_history", &args, "since")?;
        let until = parse_opt_rfc3339("query_device_state_history", &args, "until")?;
        let device_id = match args.get("device_id").and_then(|v| v.as_str()) {
            Some(s) => Some(parse_device_id("query_device_state_history", s)?),
            None => None,
        };
        let room = args
            .get("room")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let q = StateQuery {
            since,
            until,
            device_id,
            room,
            limit,
        };

        let entries = self
            .reader
            .query(&q)
            .map_err(|e| Error::Internal(format!("query failed: {e}")))?;
        let arr: Vec<Value> = entries.iter().map(state_entry_value).collect();
        Ok(json!(arr))
    }
}

// ---------- DeviceStateSnapshotAt ----------

pub struct DeviceStateSnapshotAt {
    reader: Arc<StateReader>,
    registry: Arc<DeviceRegistry>,
}

impl DeviceStateSnapshotAt {
    pub fn new(reader: Arc<StateReader>, registry: Arc<DeviceRegistry>) -> Self {
        Self { reader, registry }
    }
}

#[async_trait]
impl Tool for DeviceStateSnapshotAt {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "device_state_snapshot_at".into(),
            description: "Reconstruct the state of one or more devices at a specific point in time. Returns the most recent state entry per device whose timestamp is on or before the requested time. Use this to answer 'how were things at 8pm yesterday' and to compose set_device calls to reproduce a past scene.".into(),
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
        let at = parse_required_rfc3339("device_state_snapshot_at", &args, "at")?;

        let ids = if let Some(arr) = args.get("device_ids").and_then(|v| v.as_array()) {
            if arr.is_empty() {
                return Err(Error::InvalidArgs {
                    tool: "device_state_snapshot_at".into(),
                    reason: "device_ids must contain at least one device id".into(),
                });
            }
            arr.iter()
                .map(|v| {
                    let raw = v.as_str().ok_or_else(|| Error::InvalidArgs {
                        tool: "device_state_snapshot_at".into(),
                        reason: "device_ids must be an array of strings".into(),
                    })?;
                    parse_device_id("device_state_snapshot_at", raw)
                })
                .collect::<Result<Vec<_>>>()?
        } else if let Some(raw) = args.get("room").and_then(|v| v.as_str()) {
            let room =
                RoomName::parse(raw).map_err(|_| Error::RoomNotFound { name: raw.into() })?;
            self.registry
                .list_room(&room)
                .into_iter()
                .map(|d| d.id.clone())
                .collect()
        } else {
            return Err(Error::InvalidArgs {
                tool: "device_state_snapshot_at".into(),
                reason: "must specify one of device_ids or room".into(),
            });
        };

        let entries = self
            .reader
            .snapshot_at(at, &ids)
            .map_err(|e| Error::Internal(format!("snapshot failed: {e}")))?;
        let arr: Vec<Value> = entries.iter().map(state_entry_value).collect();
        Ok(json!(arr))
    }
}

/// Register the state-history tools onto an existing registry.
pub fn register_state_history_tools(
    reg: &mut ToolRegistry,
    reader: Arc<StateReader>,
    registry: Arc<DeviceRegistry>,
) {
    reg.register(Box::new(QueryDeviceStateHistory::new(reader.clone())));
    reg.register(Box::new(DeviceStateSnapshotAt::new(reader, registry)));
}

/// Build a `ToolRegistry` containing every device-facing Tier-1 built-in.
///
/// `LookUpCapability` is not included here because it requires an
/// `Arc<CapabilityLoader>`; callers that have one should register it
/// onto the returned registry explicitly.
pub fn default_registry<P: Publisher + 'static>(
    registry: Arc<DeviceRegistry>,
    publisher: P,
    z2m_prefix: Arc<String>,
) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(ExplainDeviceState::new(registry.clone())));
    reg.register(Box::new(GetDeviceState::new(registry.clone())));
    reg.register(Box::new(ListDevicesInRoom::new(registry.clone())));
    reg.register(Box::new(ListAllDevices::new(registry.clone())));
    reg.register(Box::new(SetDevice::new(registry, publisher, z2m_prefix)));
    reg
}

/// Build a `ToolRegistry` containing only memory + skill tools.
/// Used by `niles-bin`'s per-turn background-review fork. Excludes
/// device, weather, web_search, timer, capability, and history
/// tools — anything that could take a user-facing action or fan
/// out to external services.
pub fn restricted_registry_for_review(
    memory_store: Option<Arc<niles_memory::MemoryStore>>,
    skill_store: Option<Arc<niles_skills::SkillStore>>,
) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    if let Some(s) = memory_store {
        register_memory_tools(&mut reg, s);
    }
    if let Some(s) = skill_store {
        register_skill_tools(&mut reg, s);
    }
    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use niles_capabilities::CapabilityLoader;
    use niles_core::{Device, DeviceClass};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, content: &str) {
        fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    fn make_loader(caps: &[(&str, &str)]) -> (TempDir, Arc<CapabilityLoader>) {
        let tmp = TempDir::new().unwrap();
        for (name, content) in caps {
            let dir = tmp.path().join(name);
            fs::create_dir(&dir).unwrap();
            write_skill(&dir, content);
        }
        let loader = Arc::new(CapabilityLoader::load_from_dir(tmp.path()).unwrap());
        (tmp, loader)
    }

    fn device(id: &str, class: DeviceClass, state: DeviceState) -> Device {
        Device::new(DeviceId::parse(&format!("z2m:{id}")).unwrap(), state, class)
    }

    fn fixture_registry() -> Arc<DeviceRegistry> {
        let reg = Arc::new(DeviceRegistry::new());
        reg.upsert(device(
            "kitchen/ceiling_light",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                brightness: Some(80),
                color_temp_kelvin: Some(3000),
                ..Default::default()
            },
        ));
        reg.upsert(device(
            "living_room/floor_lamp",
            DeviceClass::Light,
            DeviceState {
                on: Some(false),
                ..Default::default()
            },
        ));
        reg.upsert(device(
            "hallway/wall_switch",
            DeviceClass::Switch,
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
        ));
        reg.upsert(device(
            "office/temp_sensor",
            DeviceClass::Sensor,
            DeviceState {
                temperature_celsius: Some(22.0),
                ..Default::default()
            },
        ));
        reg.upsert(device(
            "garage/unknown_thing",
            DeviceClass::Unknown,
            DeviceState::default(),
        ));
        reg
    }

    #[tokio::test]
    async fn get_device_state_returns_full_shape() {
        let reg = fixture_registry();
        let tool = GetDeviceState::new(reg);
        let args = json!({ "device_id": "kitchen/ceiling_light" });
        let result = tool.execute(args).await.unwrap();
        assert_eq!(result["id"], "kitchen/ceiling_light");
        assert_eq!(result["on"], true);
        assert_eq!(result["brightness"], 80);
        assert_eq!(result["color_temp_kelvin"], 3000);
        assert!(result.get("temperature_celsius").is_some());
    }

    #[tokio::test]
    async fn get_device_state_unknown_returns_device_not_found() {
        let reg = fixture_registry();
        let tool = GetDeviceState::new(reg);
        let args = json!({ "device_id": "kitchen/ghost" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::DeviceNotFound { .. }));
    }

    #[tokio::test]
    async fn get_device_state_missing_device_id_errors_invalid_args() {
        let reg = fixture_registry();
        let tool = GetDeviceState::new(reg);
        let args = json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "get_device_state"));
    }

    #[tokio::test]
    async fn get_device_state_malformed_device_id_errors_invalid_args() {
        let reg = fixture_registry();
        let tool = GetDeviceState::new(reg);
        let args = json!({ "device_id": "no_slash" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "get_device_state"));
    }

    #[tokio::test]
    async fn list_devices_in_room_returns_array_of_summaries() {
        let reg = fixture_registry();
        let tool = ListDevicesInRoom::new(reg);
        let args = json!({ "room": "kitchen" });
        let result = tool.execute(args).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "kitchen/ceiling_light");
        assert_eq!(arr[0]["on"], true);
    }

    #[tokio::test]
    async fn list_devices_in_room_empty_room_returns_empty_array() {
        let reg = fixture_registry();
        let tool = ListDevicesInRoom::new(reg);
        let args = json!({ "room": "bedroom" });
        let result = tool.execute(args).await.unwrap();
        let arr = result.as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[tokio::test]
    async fn list_devices_in_room_invalid_room_errors_room_not_found() {
        let reg = fixture_registry();
        let tool = ListDevicesInRoom::new(reg);
        let args = json!({ "room": "living room" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::RoomNotFound { name } if name == "living room"));
    }

    #[tokio::test]
    async fn list_all_devices_returns_all_summaries() {
        let reg = fixture_registry();
        let tool = ListAllDevices::new(reg);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 5);
    }

    #[tokio::test]
    async fn list_all_devices_empty_registry_returns_empty_array() {
        let reg = Arc::new(DeviceRegistry::new());
        let tool = ListAllDevices::new(reg);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result.as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn set_device_args_no_state_fields_errors_invalid_args() {
        let args = json!({ "device_id": "kitchen/ceiling_light" });
        let err = extract_set_state(&args).unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "set_device"));
    }

    #[test]
    fn set_device_args_brightness_out_of_range_errors() {
        let args = json!({ "device_id": "kitchen/ceiling_light", "brightness": 999 });
        let err = extract_set_state(&args).unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "set_device" && reason.contains("999"))
        );
    }

    #[test]
    fn extract_set_state_happy_path() {
        let args = json!({ "device_id": "kitchen/ceiling_light", "on": false, "brightness": 50 });
        let state = extract_set_state(&args).unwrap();
        assert_eq!(state.on, Some(false));
        assert_eq!(state.brightness, Some(50));
        assert_eq!(state.color_temp_kelvin, None);
    }

    #[test]
    fn set_device_missing_device_id_errors_invalid_args() {
        let args = json!({ "on": true });
        let err = required_str("set_device", &args, "device_id").unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "set_device"));
    }

    #[test]
    fn set_device_args_color_temp_kelvin_below_range_errors() {
        let args = json!({ "device_id": "kitchen/ceiling_light", "color_temp_kelvin": 500 });
        let err = extract_set_state(&args).unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "set_device" && reason.contains("500"))
        );
    }

    #[test]
    fn set_device_args_color_temp_kelvin_above_range_errors() {
        let args = json!({ "device_id": "kitchen/ceiling_light", "color_temp_kelvin": 50000 });
        let err = extract_set_state(&args).unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "set_device" && reason.contains("50000"))
        );
    }

    #[test]
    fn set_device_args_color_temp_kelvin_in_range_accepted() {
        let args = json!({ "device_id": "kitchen/ceiling_light", "color_temp_kelvin": 4000 });
        let state = extract_set_state(&args).unwrap();
        assert_eq!(state.color_temp_kelvin, Some(4000));
    }

    #[tokio::test]
    async fn look_up_capability_known_name_returns_full_metadata_and_body() {
        let (_tmp, loader) = make_loader(&[(
            "lights",
            "---\nname: lights\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lights\n\nTurn on/off lights.\n",
        )]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "lights" })).await.unwrap();

        assert_eq!(result["found"], true);
        assert_eq!(result["name"], "lights");
        assert_eq!(result["description"], "Control smart lights");
        assert_eq!(result["version"], "1.0.0");
        assert_eq!(result["body"], "# Lights\n\nTurn on/off lights.\n");
    }

    #[tokio::test]
    async fn look_up_capability_unknown_name_returns_available_names() {
        let (_tmp, loader) = make_loader(&[
            (
                "alpha",
                "---\nname: alpha\ndescription: Alpha cap\nversion: 1.0.0\n---\nAlpha body.\n",
            ),
            (
                "zebra",
                "---\nname: zebra\ndescription: Zebra cap\nversion: 1.0.0\n---\nZebra body.\n",
            ),
        ]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "missing" })).await.unwrap();

        assert_eq!(result["found"], false);
        assert_eq!(result["name"], "missing");
        let available = result["available"].as_array().unwrap();
        assert_eq!(available.len(), 2);
        assert_eq!(available[0], "alpha");
        assert_eq!(available[1], "zebra");
    }

    #[tokio::test]
    async fn look_up_capability_missing_name_errors_invalid_args() {
        let (_tmp, loader) = make_loader(&[]);
        let tool = LookUpCapability::new(loader);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "look_up_capability"));
    }

    #[tokio::test]
    async fn look_up_capability_non_string_name_errors_invalid_args() {
        let (_tmp, loader) = make_loader(&[]);
        let tool = LookUpCapability::new(loader);
        let err = tool.execute(json!({ "name": 42 })).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "look_up_capability"));
    }

    #[tokio::test]
    async fn look_up_capability_empty_string_name_returns_not_found() {
        let (_tmp, loader) = make_loader(&[(
            "lights",
            "---\nname: lights\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lights\n\nTurn on/off lights.\n",
        )]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "" })).await.unwrap();

        assert_eq!(result["found"], false);
        assert_eq!(result["name"], "");
        let available = result["available"].as_array().unwrap();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0], "lights");
    }

    #[tokio::test]
    async fn look_up_capability_empty_registry_returns_not_found_empty_available() {
        let (_tmp, loader) = make_loader(&[]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "anything" })).await.unwrap();

        assert_eq!(result["found"], false);
        assert_eq!(result["name"], "anything");
        let available = result["available"].as_array().unwrap();
        assert!(available.is_empty());
    }

    #[tokio::test]
    async fn look_up_capability_hit_does_not_leak_others() {
        let (_tmp, loader) = make_loader(&[
            (
                "cap-a",
                "---\nname: cap-a\ndescription: Cap A\nversion: 1.0.0\n---\nBody A.\n",
            ),
            (
                "cap-b",
                "---\nname: cap-b\ndescription: Cap B\nversion: 1.0.0\n---\nBody B.\n",
            ),
        ]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "cap-a" })).await.unwrap();

        assert_eq!(result["found"], true);
        assert_eq!(result["name"], "cap-a");
        assert_eq!(result["body"], "Body A.\n");
        assert!(result.get("available").is_none());
    }

    #[tokio::test]
    async fn look_up_capability_prerequisites_roundtrip() {
        let (_tmp, loader) = make_loader(&[(
            "prereq",
            "---\nname: prereq\ndescription: Needs deps\nversion: 1.0.0\nprerequisites:\n  - foo\n  - bar\n---\nBody.\n",
        )]);
        let tool = LookUpCapability::new(loader);
        let result = tool.execute(json!({ "name": "prereq" })).await.unwrap();

        assert_eq!(result["found"], true);
        let prereqs = result["prerequisites"].as_array().unwrap();
        assert_eq!(prereqs.len(), 2);
        assert_eq!(prereqs[0], "foo");
        assert_eq!(prereqs[1], "bar");
    }

    #[derive(Clone, Default)]
    struct MockPublisher {
        topics: Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Publisher for MockPublisher {
        async fn publish(&self, topic: &str, _payload: Vec<u8>) -> niles_mqtt::Result<()> {
            self.topics.lock().await.push(topic.to_string());
            Ok(())
        }
    }

    fn set_device_setup() -> (MockPublisher, SetDevice<MockPublisher>) {
        let mock = MockPublisher::default();
        let tool = SetDevice::new(fixture_registry(), mock.clone(), Arc::new("z2m".into()));
        (mock, tool)
    }

    #[tokio::test]
    async fn set_device_light_publishes_and_returns_ok() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "kitchen/ceiling_light", "on": false });
        let result = tool.execute(args).await.unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["topic"], "z2m/kitchen/ceiling_light/set");
        let topics = mock.topics.lock().await;
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0], "z2m/kitchen/ceiling_light/set");
    }

    #[tokio::test]
    async fn set_device_switch_returns_wrong_device_class() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "hallway/wall_switch", "on": true });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::WrongDeviceClass { id, class } if id == "hallway/wall_switch" && class == DeviceClass::Switch)
        );
        assert!(mock.topics.lock().await.is_empty());
    }

    #[tokio::test]
    async fn set_device_sensor_returns_wrong_device_class() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "office/temp_sensor", "on": true });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::WrongDeviceClass { id, class } if id == "office/temp_sensor" && class == DeviceClass::Sensor)
        );
        assert!(mock.topics.lock().await.is_empty());
    }

    #[tokio::test]
    async fn set_device_unknown_returns_wrong_device_class() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "garage/unknown_thing", "on": true });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::WrongDeviceClass { id, class } if id == "garage/unknown_thing" && class == DeviceClass::Unknown)
        );
        assert!(mock.topics.lock().await.is_empty());
    }

    #[tokio::test]
    async fn set_device_missing_id_returns_device_not_found() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "nonexistent/device", "on": true });
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::DeviceNotFound { id } if id == "nonexistent/device"));
        assert!(mock.topics.lock().await.is_empty());
    }

    #[tokio::test]
    async fn set_device_sensor_with_no_args_returns_wrong_device_class_before_invalid_args() {
        let (mock, tool) = set_device_setup();
        let args = json!({ "device_id": "office/temp_sensor" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::WrongDeviceClass { id, class } if id == "office/temp_sensor" && class == DeviceClass::Sensor)
        );
        assert!(mock.topics.lock().await.is_empty());
    }

    // ---------- explain_device_state tests ----------

    #[test]
    fn explain_light_full_state() {
        let d = device(
            "office/lightstrip",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                brightness: Some(100),
                color_temp_kelvin: Some(2857),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/lightstrip is on at 100% brightness, color temperature 2857K"
        );
    }

    #[test]
    fn explain_light_no_kelvin() {
        let d = device(
            "office/lightstrip",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                brightness: Some(80),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/lightstrip is on at 80% brightness"
        );
    }

    #[test]
    fn explain_light_on_only() {
        let d = device(
            "office/lightstrip",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(explain_device(&d), "office/lightstrip is on");
    }

    #[test]
    fn explain_light_kelvin_without_brightness_does_not_emit_hanging_comma() {
        let d = device(
            "office/lightstrip",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                color_temp_kelvin: Some(2857),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/lightstrip is on at color temperature 2857K"
        );
    }

    #[test]
    fn explain_light_off_ignores_brightness() {
        let d = device(
            "living_room/floor_lamp",
            DeviceClass::Light,
            DeviceState {
                on: Some(false),
                brightness: Some(50),
                ..Default::default()
            },
        );
        assert_eq!(explain_device(&d), "living_room/floor_lamp is off");
    }

    #[test]
    fn explain_light_no_state_says_not_reported() {
        let d = device(
            "bedroom/light",
            DeviceClass::Light,
            DeviceState {
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "bedroom/light is a light but its state hasn't been reported yet"
        );
    }

    #[test]
    fn explain_light_zero_brightness_does_not_collapse_to_off() {
        let d = device(
            "office/lightstrip",
            DeviceClass::Light,
            DeviceState {
                on: Some(true),
                brightness: Some(0),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/lightstrip is on at 0% brightness"
        );
    }

    #[test]
    fn explain_switch_with_battery() {
        let d = device(
            "office/switch",
            DeviceClass::Switch,
            DeviceState {
                battery_percent: Some(100),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/switch is a button device; battery 100%"
        );
    }

    #[test]
    fn explain_switch_no_battery_report() {
        let d = device(
            "office/switch",
            DeviceClass::Switch,
            DeviceState {
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "office/switch is a button device; no battery report yet"
        );
    }

    #[test]
    fn explain_sensor_all_fields() {
        let d = device(
            "kitchen/thermometer",
            DeviceClass::Sensor,
            DeviceState {
                temperature_celsius: Some(21.5),
                humidity_percent: Some(47.0),
                battery_percent: Some(88),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "kitchen/thermometer is a sensor; temperature 21.5°C, humidity 47%, battery 88%"
        );
    }

    #[test]
    fn explain_sensor_single_field_no_comma_drift() {
        let d = device(
            "kitchen/thermometer",
            DeviceClass::Sensor,
            DeviceState {
                temperature_celsius: Some(21.5),
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "kitchen/thermometer is a sensor; temperature 21.5°C"
        );
    }

    #[test]
    fn explain_sensor_no_readings() {
        let d = device(
            "kitchen/thermometer",
            DeviceClass::Sensor,
            DeviceState {
                ..Default::default()
            },
        );
        assert_eq!(
            explain_device(&d),
            "kitchen/thermometer is a sensor but no readings have been reported yet"
        );
    }

    #[test]
    fn explain_unknown_falls_back_to_generic() {
        let d = device(
            "garage/unknown_thing",
            DeviceClass::Unknown,
            DeviceState {
                on: Some(true),
                brightness: Some(50),
                ..Default::default()
            },
        );
        let s = explain_device(&d);
        assert_eq!(
            s,
            "garage/unknown_thing is an unclassified device; on: true, brightness: 50"
        );
    }

    #[tokio::test]
    async fn explain_device_state_wraps_sentence_under_explanation_key() {
        let reg = fixture_registry();
        let tool = ExplainDeviceState::new(reg);
        let args = json!({ "device_id": "kitchen/ceiling_light" });
        let result = tool.execute(args).await.unwrap();
        assert_eq!(
            result,
            json!({
                "explanation": "kitchen/ceiling_light is on at 80% brightness, color temperature 3000K"
            })
        );
    }

    #[tokio::test]
    async fn explain_device_state_unknown_id() {
        let reg = fixture_registry();
        let tool = ExplainDeviceState::new(reg);
        let args = json!({ "device_id": "kitchen/ghost" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::DeviceNotFound { id } if id == "kitchen/ghost"));
    }

    #[tokio::test]
    async fn explain_device_state_missing_device_id() {
        let reg = fixture_registry();
        let tool = ExplainDeviceState::new(reg);
        let args = json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "explain_device_state"));
    }

    #[test]
    fn default_registry_includes_explain_device_state() {
        let reg = fixture_registry();
        let tools = default_registry(reg, MockPublisher::default(), Arc::new("z2m".into()));
        let names: Vec<String> = tools.llm_tools().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"explain_device_state".to_string()));
    }

    // ---------- timer tool tests ----------

    fn localhost() -> std::net::SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    #[tokio::test]
    async fn list_timers_empty_store_returns_empty_array() {
        let store = Arc::new(TimerStore::new());
        let tool = ListTimers::new(store);
        let result = tool.execute(json!({})).await.unwrap();
        assert_eq!(result, json!({ "timers": [] }));
    }

    #[tokio::test]
    async fn list_timers_returns_both_entries_with_correct_shape() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        store.set(
            std::time::Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        store.set(
            std::time::Duration::from_secs(120),
            Some("laundry".into()),
            localhost(),
            now,
        );
        let tool = ListTimers::new(store);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result["timers"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // Sorted by expires_at — pasta (60s) first
        assert_eq!(arr[0]["name"], "pasta");
        assert_eq!(arr[0]["duration_seconds"], 60);
        assert_eq!(arr[0]["state"], "pending");
        assert!(arr[0]["remaining_seconds"].as_i64().unwrap() >= 0);
        assert_eq!(arr[1]["name"], "laundry");
        assert_eq!(arr[1]["duration_seconds"], 120);
    }

    #[tokio::test]
    async fn cancel_timer_existing_returns_count_one() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        store.set(
            std::time::Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        let tool = CancelTimer::new(store.clone());
        let result = tool.execute(json!({ "name": "pasta" })).await.unwrap();
        assert_eq!(result, json!({ "cancelled": 1, "name": "pasta" }));
        assert_eq!(store.list().len(), 0);
    }

    #[tokio::test]
    async fn cancel_timer_missing_returns_count_zero() {
        let store = Arc::new(TimerStore::new());
        let tool = CancelTimer::new(store);
        let result = tool.execute(json!({ "name": "missing" })).await.unwrap();
        assert_eq!(result, json!({ "cancelled": 0, "name": "missing" }));
    }

    #[tokio::test]
    async fn cancel_timer_is_case_insensitive() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        store.set(
            std::time::Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        let tool = CancelTimer::new(store.clone());
        let result = tool.execute(json!({ "name": " Pasta " })).await.unwrap();
        assert_eq!(result["cancelled"], 1);
        assert_eq!(result["name"], "pasta"); // echoed canonical, not the raw input
        assert_eq!(store.list().len(), 0);
    }

    #[tokio::test]
    async fn cancel_timer_missing_name_arg_returns_invalid_args() {
        let store = Arc::new(TimerStore::new());
        let tool = CancelTimer::new(store);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "cancel_timer"));
    }

    #[tokio::test]
    async fn get_timer_remaining_existing_returns_found_true() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        store.set(
            std::time::Duration::from_secs(300),
            Some("pasta".into()),
            localhost(),
            now,
        );
        let tool = GetTimerRemaining::new(store);
        let result = tool.execute(json!({ "name": "pasta" })).await.unwrap();
        assert_eq!(result["found"], true);
        assert_eq!(result["name"], "pasta");
        assert_eq!(result["state"], "pending");
        let remaining = result["remaining_seconds"].as_i64().unwrap();
        assert!((0..=300).contains(&remaining));
    }

    #[tokio::test]
    async fn get_timer_remaining_missing_returns_found_false_null_remaining() {
        let store = Arc::new(TimerStore::new());
        let tool = GetTimerRemaining::new(store);
        let result = tool.execute(json!({ "name": "missing" })).await.unwrap();
        assert_eq!(
            result,
            json!({
                "found": false,
                "name": "missing",
                "remaining_seconds": null,
            })
        );
    }

    #[tokio::test]
    async fn get_timer_remaining_is_case_insensitive() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        store.set(
            std::time::Duration::from_secs(60),
            Some("Pasta".into()),
            localhost(),
            now,
        );
        let tool = GetTimerRemaining::new(store);
        let result = tool.execute(json!({ "name": " PASTA " })).await.unwrap();
        assert_eq!(result["found"], true);
        assert_eq!(result["name"], "pasta"); // stored canonicalized
    }

    #[tokio::test]
    async fn get_timer_remaining_missing_name_arg_returns_invalid_args() {
        let store = Arc::new(TimerStore::new());
        let tool = GetTimerRemaining::new(store);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(matches!(err, Error::InvalidArgs { tool, .. } if tool == "get_timer_remaining"));
    }

    #[tokio::test]
    async fn list_timers_reports_ringing_state() {
        let store = Arc::new(TimerStore::new());
        let now = chrono::Utc::now();
        let id = store.set(
            std::time::Duration::from_secs(60),
            Some("pasta".into()),
            localhost(),
            now,
        );
        store.mark_ringing(id);
        let tool = ListTimers::new(store);
        let result = tool.execute(json!({})).await.unwrap();
        let arr = result["timers"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["state"], "ringing");
    }

    #[tokio::test]
    async fn get_timer_remaining_missing_echoes_canonical_name() {
        let store = Arc::new(TimerStore::new());
        let tool = GetTimerRemaining::new(store);
        let result = tool.execute(json!({ "name": " MISSING " })).await.unwrap();
        assert_eq!(result["found"], false);
        assert_eq!(result["name"], "missing");
        assert!(result["remaining_seconds"].is_null());
    }

    #[tokio::test]
    async fn register_timer_tools_registers_all_three_names() {
        let store = Arc::new(TimerStore::new());
        let mut reg = ToolRegistry::new();
        register_timer_tools(&mut reg, store);
        let names: Vec<String> = reg.llm_tools().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"list_timers".to_string()));
        assert!(names.contains(&"cancel_timer".to_string()));
        assert!(names.contains(&"get_timer_remaining".to_string()));
    }

    // ---------- state history tool tests ----------

    #[tokio::test]
    async fn query_device_state_history_rejects_invalid_since() {
        let reader = Arc::new(StateReader::disabled());
        let tool = QueryDeviceStateHistory::new(reader);
        let args = json!({ "since": "not-a-timestamp" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool: t, .. } if t == "query_device_state_history")
        );
    }

    #[tokio::test]
    async fn device_state_snapshot_at_neither_ids_nor_room_errors() {
        let reader = Arc::new(StateReader::disabled());
        let reg = fixture_registry();
        let tool = DeviceStateSnapshotAt::new(reader, reg);
        let args = json!({ "at": "2026-01-01T12:00:00Z" });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool: t, reason } if t == "device_state_snapshot_at" && reason.contains("device_ids or room"))
        );
    }

    #[tokio::test]
    async fn device_state_snapshot_at_empty_device_ids_errors() {
        let reader = Arc::new(StateReader::disabled());
        let reg = fixture_registry();
        let tool = DeviceStateSnapshotAt::new(reader, reg);
        let args = json!({ "at": "2026-01-01T12:00:00Z", "device_ids": [] });
        let err = tool.execute(args).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool: t, reason } if t == "device_state_snapshot_at" && reason.contains("at least one"))
        );
    }

    #[tokio::test]
    async fn device_state_snapshot_at_room_expands_to_registered_devices() {
        let tmp = tempfile::TempDir::new().unwrap();
        let writer = niles_history::StateWriter::new(tmp.path()).unwrap();
        let reader = Arc::new(StateReader::new(tmp.path()));
        let reg = fixture_registry();

        let t = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        writer
            .append(&niles_history::StateEntry {
                ts: t,
                device_id: DeviceId::parse("z2m:living_room/floor_lamp").unwrap(),
                state: DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            })
            .unwrap();

        let tool = DeviceStateSnapshotAt::new(reader, reg);
        let args = json!({ "at": "2026-01-01T12:00:00Z", "room": "living_room" });
        let result = tool.execute(args).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["device_id"], "living_room/floor_lamp");
    }

    #[tokio::test]
    async fn query_device_state_history_returns_formatted_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let writer = niles_history::StateWriter::new(tmp.path()).unwrap();
        let reader = Arc::new(StateReader::new(tmp.path()));

        let t = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        writer
            .append(&niles_history::StateEntry {
                ts: t,
                device_id: DeviceId::parse("z2m:kitchen/ceiling_light").unwrap(),
                state: DeviceState {
                    on: Some(true),
                    brightness: Some(80),
                    ..Default::default()
                },
            })
            .unwrap();

        let tool = QueryDeviceStateHistory::new(reader);
        let args = json!({ "since": "2026-01-01T00:00:00Z", "until": "2026-01-01T23:59:59Z" });
        let result = tool.execute(args).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["device_id"], "kitchen/ceiling_light");
        assert_eq!(arr[0]["state"]["on"], true);
        assert_eq!(arr[0]["state"]["brightness"], 80);
    }

    #[tokio::test]
    async fn device_state_snapshot_at_with_device_ids_returns_formatted_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let writer = niles_history::StateWriter::new(tmp.path()).unwrap();
        let reader = Arc::new(StateReader::new(tmp.path()));
        let reg = fixture_registry();

        let t = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        writer
            .append(&niles_history::StateEntry {
                ts: t,
                device_id: DeviceId::parse("z2m:kitchen/ceiling_light").unwrap(),
                state: DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            })
            .unwrap();

        let tool = DeviceStateSnapshotAt::new(reader, reg);
        let args = json!({
            "at": "2026-01-01T12:00:00Z",
            "device_ids": ["kitchen/ceiling_light"]
        });
        let result = tool.execute(args).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["device_id"], "kitchen/ceiling_light");
        assert_eq!(arr[0]["state"]["on"], true);
    }

    #[test]
    fn register_state_history_tools_registers_both_names() {
        let reader = Arc::new(StateReader::disabled());
        let reg = fixture_registry();
        let mut registry = ToolRegistry::new();
        register_state_history_tools(&mut registry, reader, reg);
        let names: Vec<String> = registry.llm_tools().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"query_device_state_history".to_string()));
        assert!(names.contains(&"device_state_snapshot_at".to_string()));
    }

    #[test]
    fn restricted_registry_for_review_with_both_stores_has_exact_tools() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(
            niles_memory::MemoryStore::open(niles_memory::MemoryConfig {
                directory: tmp.path().to_path_buf(),
                ..Default::default()
            })
            .unwrap(),
        );
        let skill_tmp = TempDir::new().unwrap();
        let skill =
            Arc::new(niles_skills::SkillStore::open(skill_tmp.path(), 100_000, 1_048_576).unwrap());
        let reg = restricted_registry_for_review(Some(memory), Some(skill));
        let names: std::collections::HashSet<String> =
            reg.llm_tools().into_iter().map(|t| t.name).collect();
        let expected: std::collections::HashSet<String> = [
            "memory",
            "mint_skill",
            "patch_skill",
            "delete_skill",
            "view_skill",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn restricted_registry_for_review_with_none_is_empty() {
        let reg = restricted_registry_for_review(None, None);
        assert!(reg.llm_tools().is_empty());
    }

    #[test]
    fn restricted_registry_for_review_excludes_device_tools() {
        let tmp = TempDir::new().unwrap();
        let memory = Arc::new(
            niles_memory::MemoryStore::open(niles_memory::MemoryConfig {
                directory: tmp.path().to_path_buf(),
                ..Default::default()
            })
            .unwrap(),
        );
        let skill_tmp = TempDir::new().unwrap();
        let skill =
            Arc::new(niles_skills::SkillStore::open(skill_tmp.path(), 100_000, 1_048_576).unwrap());
        let reg = restricted_registry_for_review(Some(memory), Some(skill));
        let names: Vec<String> = reg.llm_tools().into_iter().map(|t| t.name).collect();
        for excluded in [
            "get_device_state",
            "explain_device_state",
            "list_devices_in_room",
            "list_all_devices",
            "set_device",
            "look_up_capability",
            "query_command_history",
            "query_device_state_history",
            "device_state_snapshot_at",
            "list_timers",
            "cancel_timer",
            "get_timer_remaining",
            "weather",
            "web_search",
        ] {
            assert!(
                !names.contains(&excluded.to_string()),
                "restricted registry should not contain '{}'",
                excluded
            );
        }
    }
}

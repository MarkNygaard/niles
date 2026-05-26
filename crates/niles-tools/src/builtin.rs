//! Built-in LLM tools: device read/write + room/global listing.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_capabilities::CapabilityLoader;
use niles_core::{DeviceClass, DeviceId, DeviceRegistry, DeviceState, RoomName};
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
        let count = self.timers.cancel_by_name(name);
        Ok(json!({ "cancelled": count, "name": name }))
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

#[cfg(test)]
mod tests {
    use super::*;
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
        Device {
            id: DeviceId::parse(&format!("z2m:{id}")).unwrap(),
            state,
            class,
        }
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
}

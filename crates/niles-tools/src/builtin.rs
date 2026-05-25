//! Built-in LLM tools: device read/write + room/global listing.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_capabilities::CapabilityLoader;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, RoomName};
use niles_mqtt::{MqttPublisher, format_set_command, is_actionable};
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
    json!({
        "id": format!("{}/{}", device.id.room(), device.id.name()),
        "on": device.state.on,
        "brightness": device.state.brightness,
        "color_temp_kelvin": device.state.color_temp_kelvin,
        "temperature_celsius": device.state.temperature_celsius,
        "humidity_percent": device.state.humidity_percent,
        "battery_percent": device.state.battery_percent,
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

// ---------- SetDevice ----------

pub struct SetDevice {
    registry: Arc<DeviceRegistry>,
    publisher: MqttPublisher,
    z2m_prefix: Arc<String>,
}

impl SetDevice {
    pub fn new(
        registry: Arc<DeviceRegistry>,
        publisher: MqttPublisher,
        z2m_prefix: Arc<String>,
    ) -> Self {
        Self {
            registry,
            publisher,
            z2m_prefix,
        }
    }
}

#[async_trait]
impl Tool for SetDevice {
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

        if self.registry.get(&id).is_none() {
            return Err(Error::DeviceNotFound { id: raw.into() });
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
        match self.loader.get(name) {
            Some(cap) => Ok(json!({
                "found": true,
                "name": cap.metadata.name,
                "description": cap.metadata.description,
                "version": cap.metadata.version,
                "prerequisites": cap.metadata.prerequisites,
                "body": cap.body
            })),
            None => Ok(json!({
                "found": false,
                "name": name,
                "available": self.loader.names()
            })),
        }
    }
}

/// Build a `ToolRegistry` containing every device-facing Tier-1 built-in.
///
/// `LookUpCapability` is not included here because it requires an
/// `Arc<CapabilityLoader>`; callers that have one should register it
/// onto the returned registry explicitly.
pub fn default_registry(
    registry: Arc<DeviceRegistry>,
    publisher: MqttPublisher,
    z2m_prefix: Arc<String>,
) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
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

    fn fixture_registry() -> Arc<DeviceRegistry> {
        let reg = Arc::new(DeviceRegistry::new());
        let kitchen_light = Device {
            id: DeviceId::parse("z2m:kitchen/ceiling_light").unwrap(),
            state: DeviceState {
                on: Some(true),
                brightness: Some(80),
                color_temp_kelvin: Some(3000),
                ..Default::default()
            },
            class: DeviceClass::Light,
        };
        let living_lamp = Device {
            id: DeviceId::parse("z2m:living_room/floor_lamp").unwrap(),
            state: DeviceState {
                on: Some(false),
                ..Default::default()
            },
            class: DeviceClass::Light,
        };
        reg.upsert(kitchen_light);
        reg.upsert(living_lamp);
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
        assert_eq!(arr.len(), 2);
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
}

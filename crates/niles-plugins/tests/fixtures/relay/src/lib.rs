//! A small interface 0.2 plugin for the MQTT tests.
//!
//! `init` connects to the broker its `broker` setting names and
//! subscribes to `relay/in/#`. Each message there that is a number
//! becomes the `echo` reading on `office/relay`, and every one is
//! published back on `relay/out/<the rest>`. Tools let a test see the
//! connection's statuses and do what a plugin can: publish, subscribe,
//! accept a changed certificate, disconnect.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::mqtt::{self, Qos};
use niles::integration::types::{Device, DeviceClass, LightCapabilities, Reading};
use niles::integration::{config, devices};
use serde_json::{Value, json};
use std::sync::Mutex;

struct Relay;

export!(Relay);

const DEVICE: &str = "office/relay";

static CONN: Mutex<Option<u32>> = Mutex::new(None);
static STATUSES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn text(key: &str) -> Option<String> {
    match config::get(key) {
        Some(config::SettingValue::Text(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn empty_state() -> DeviceState {
    DeviceState {
        on: None,
        brightness: None,
        color_temp_kelvin: None,
        rgb: None,
        temperature_celsius: None,
        humidity_percent: None,
        battery_percent: None,
        open: None,
        motion: None,
        illuminance_lux: None,
        position: None,
        readings: Vec::new(),
    }
}

fn conn() -> Result<u32, String> {
    CONN.lock().unwrap().ok_or_else(|| "Not connected.".to_string())
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

impl Guest for Relay {
    fn init() -> Result<(), String> {
        devices::upsert(&Device {
            id: DEVICE.into(),
            class: DeviceClass::Sensor,
            light: LightCapabilities {
                color_temp: false,
                rgb: false,
            },
            controllable: false,
            state: empty_state(),
        })
        .map_err(|e| format!("{e:?}"))?;
        let broker = text("broker").unwrap_or_else(|| "plain".into());
        let conn = mqtt::connect(
            &broker,
            &mqtt::ConnectOptions {
                client_id: Some("relay".into()),
                username: text("username"),
                password: config::secret("password"),
                keep_alive: Some(5),
                clean_session: None,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
        mqtt::subscribe(conn, "relay/in/#", Qos::AtLeastOnce, false)
            .map_err(|e| format!("{e:?}"))?;
        *CONN.lock().unwrap() = Some(conn);
        Ok(())
    }

    fn tools() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str| ToolDef {
            name: name.into(),
            description: description.into(),
            schema: json!({ "type": "object", "properties": {} }).to_string(),
        };
        vec![
            tool("statuses", "What the connection said, in order."),
            tool("received", "The topics received, in order."),
            tool("publish", "Publish a message."),
            tool("subscribe", "Subscribe to a filter."),
            tool("accept", "Accept a changed certificate."),
            tool("disconnect", "Close the connection."),
        ]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        match name.as_str() {
            "statuses" => Ok(json!(*STATUSES.lock().unwrap()).to_string()),
            "received" => Ok(json!(*RECEIVED.lock().unwrap()).to_string()),
            "publish" => mqtt::publish(
                conn()?,
                arg(&args, "topic").ok_or("Which topic?")?,
                arg(&args, "payload").unwrap_or_default().as_bytes(),
                Qos::AtLeastOnce,
                false,
            )
            .map(|()| "published".to_string())
            .map_err(|e| format!("{e:?}")),
            "subscribe" => mqtt::subscribe(
                conn()?,
                arg(&args, "filter").ok_or("Which filter?")?,
                Qos::AtMostOnce,
                args.get("latest_only").and_then(Value::as_bool) == Some(true),
            )
            .map(|()| "subscribed".to_string())
            .map_err(|e| format!("{e:?}")),
            "accept" => mqtt::accept_certificate(conn()?)
                .map(|()| "accepted".to_string())
                .map_err(|e| format!("{e:?}")),
            "disconnect" => {
                mqtt::disconnect(conn()?);
                Ok("disconnected".into())
            }
            other => Err(format!("There is no tool called {other}.")),
        }
    }

    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Err("The relay cannot be switched.".into())
    }

    fn on_timer(_id: String) {}

    fn on_event(_event: Event) {}

    fn render(surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        Err(format!("There is no page called {surface_id}."))
    }

    fn on_action(
        _surface_id: String,
        _control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("There are no controls.".into())
    }

    fn on_mqtt_message(conn: u32, topic: String, payload: Vec<u8>) {
        RECEIVED.lock().unwrap().push(topic.clone());
        let Some(rest) = topic.strip_prefix("relay/in/") else {
            return;
        };
        if let Some(value) = std::str::from_utf8(&payload)
            .ok()
            .and_then(|text| text.trim().parse::<f64>().ok())
        {
            let mut state = empty_state();
            state.readings = vec![Reading {
                name: "echo".into(),
                value,
                unit: None,
            }];
            let _ = devices::report_state(DEVICE, &state);
        }
        let _ = mqtt::publish(
            conn,
            &format!("relay/out/{rest}"),
            &payload,
            Qos::AtLeastOnce,
            false,
        );
    }

    fn on_mqtt_status(_conn: u32, status: ConnectionStatus) {
        let said = match status {
            ConnectionStatus::Connected => "connected".to_string(),
            ConnectionStatus::Disconnected(why) => format!("disconnected: {why}"),
            ConnectionStatus::Failed(why) => format!("failed: {why}"),
            ConnectionStatus::CertificateChanged(change) => {
                format!("certificate-changed {} {}", change.trusted, change.presented)
            }
        };
        let _ = devices::set_available(DEVICE, said == "connected");
        STATUSES.lock().unwrap().push(said);
    }

    fn stop() {}
}

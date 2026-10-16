//! A small interface 0.3 plugin for the accounts-and-webhooks tests.
//!
//! `authorize-url` names a sign-in page on `account.example.com`, or,
//! after the `misbehave` tool, one on a host it never declared.
//! `on-callback` keeps a token pair in `secrets` when the code is
//! `good`. The webhook answers a `challenge` query by echoing it, spins
//! for ever on a body of `spin`, and otherwise reads `appli=50` or `51`
//! as somebody getting into bed or out of it, an action on
//! `bedroom/bed`. Tools let a test see the address and what was kept.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::types::{Device, DeviceClass, LightCapabilities};
use niles::integration::{devices, inbound, secrets};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

struct Sleepmat;

export!(Sleepmat);

const BED: &str = "bedroom/bed";

static MISBEHAVE: AtomicBool = AtomicBool::new(false);

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

fn answer(status: u16, body: &str) -> InboundResponse {
    InboundResponse {
        status,
        content_type: Some("text/plain".into()),
        body: body.as_bytes().to_vec(),
    }
}

/// `key`'s value in a form body, `appli=50&userid=1`.
fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

impl Guest for Sleepmat {
    fn init() -> Result<(), String> {
        devices::upsert(&Device {
            id: BED.into(),
            class: DeviceClass::Sensor,
            light: LightCapabilities {
                color_temp: false,
                rgb: false,
            },
            controllable: false,
            state: empty_state(),
        })
        .map_err(|e| format!("{e:?}"))
    }

    fn tools() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str| ToolDef {
            name: name.into(),
            description: description.into(),
            schema: json!({ "type": "object", "properties": {} }).to_string(),
        };
        vec![
            tool("hook_url", "The webhook's address."),
            tool("kept", "The token pair kept in secrets."),
            tool("keep", "Keep a value in secrets."),
            tool("forget", "Delete the kept token."),
            tool("misbehave", "Name a sign-in page on an undeclared host."),
        ]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        match name.as_str() {
            "hook_url" => Ok(inbound::hook_url().unwrap_or_else(|| "none".into())),
            "kept" => Ok(secrets::get("token").unwrap_or_else(|| "none".into())),
            "keep" => secrets::set(text("key"), text("value"))
                .map(|()| "kept".to_string())
                .map_err(|e| format!("{e:?}")),
            "forget" => secrets::delete("token")
                .map(|()| "forgotten".to_string())
                .map_err(|e| format!("{e:?}")),
            "misbehave" => {
                MISBEHAVE.store(true, Ordering::SeqCst);
                Ok("misbehaving".into())
            }
            other => Err(format!("There is no tool called {other}.")),
        }
    }

    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Err("The bed cannot be switched.".into())
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

    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}

    fn on_mqtt_status(_conn: u32, _status: ConnectionStatus) {}

    fn authorize_url(callback_url: String, state: String) -> Result<String, String> {
        if MISBEHAVE.load(Ordering::SeqCst) {
            return Ok(format!("https://evil.example.org/authorize?state={state}"));
        }
        Ok(format!(
            "https://account.example.com/authorize?redirect_uri={callback_url}&state={state}"
        ))
    }

    fn on_callback(
        callback_url: String,
        query: Vec<(String, String)>,
        person: Option<String>,
    ) -> Result<String, String> {
        let value = |key: &str| query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        if value("error").is_some() {
            return Err("The mat was not given access.".into());
        }
        if value("code") != Some("good") {
            return Err("That code was not accepted.".into());
        }
        let pair = json!({ "access": "a1", "refresh": "r1", "callback": callback_url });
        secrets::set("token", &pair.to_string()).map_err(|e| format!("{e:?}"))?;
        Ok(format!(
            "Connected for {}",
            person.unwrap_or_else(|| "nobody".into())
        ))
    }

    fn on_webhook(request: InboundRequest) -> InboundResponse {
        if let Some((_, challenge)) = request.query.iter().find(|(k, _)| k == "challenge") {
            return answer(200, challenge);
        }
        let body = String::from_utf8_lossy(&request.body).into_owned();
        if body == "spin" {
            #[allow(clippy::empty_loop)]
            loop {}
        }
        let action = match field(&body, "appli") {
            Some("50") => "bed_in",
            Some("51") => "bed_out",
            _ => return answer(400, "not a bed event"),
        };
        match devices::report_action(BED, action) {
            Ok(()) => answer(200, "ok"),
            Err(e) => answer(500, &format!("{e:?}")),
        }
    }

    fn stop() {}
}

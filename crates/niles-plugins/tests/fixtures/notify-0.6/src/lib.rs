wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::notify;
use serde_json::{Value, json};

struct NotificationFixture;
export!(NotificationFixture);

impl Guest for NotificationFixture {
    fn init() -> Result<(), String> {
        Ok(())
    }

    fn tools() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "request".into(),
                description: "Accept a tracked notification request and return its ID.".into(),
                schema: json!({"type":"object","properties":{"message":{"type":"string"},"room":{"type":"string"}}}).to_string(),
            },
            ToolDef {
                name: "status".into(),
                description: "Read an accepted notification request's transport status.".into(),
                schema: json!({"type":"object","properties":{"request":{"type":"integer"}}}).to_string(),
            },
        ]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        match name.as_str() {
            "request" => {
                let message = args
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Fixture notification.");
                let room = args.get("room").and_then(Value::as_str).map(str::to_owned);
                let priority = match args.get("priority").and_then(Value::as_str) {
                    Some("low") => notify::Priority::Low,
                    Some("urgent") => notify::Priority::Urgent,
                    _ => notify::Priority::Normal,
                };
                let id = notify::request(message, priority, room.as_deref())
                    .map_err(|e| format!("request: {e:?}"))?;
                Ok(json!({"request":id}).to_string())
            }
            "status" => {
                let id = args
                    .get("request")
                    .and_then(Value::as_u64)
                    .ok_or("request must be an unsigned integer")?;
                let status = notify::status(id).map_err(|e| format!("status: {e:?}"))?;
                Ok(json!({"status":format!("{status:?}")}).to_string())
            }
            _ => Err(format!("unknown tool {name}")),
        }
    }

    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Ok(())
    }
    fn on_timer(_id: String) {}
    fn on_event(_event: Event) {}
    fn render(_surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        Err("no UI".into())
    }
    fn on_action(
        _surface_id: String,
        _control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("no UI".into())
    }
    fn stop() {}
    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}
    fn on_mqtt_status(_conn: u32, _status: niles::integration::mqtt::ConnectionStatus) {}
    fn on_websocket_message(_conn: u32, _message: niles::integration::websocket::Message) {}
    fn on_websocket_status(_conn: u32, _status: niles::integration::websocket::ConnectionStatus) {}
    fn authorize_url(_callback_url: String, _state: String) -> Result<String, String> {
        Err("no account".into())
    }
    fn on_callback(
        _callback_url: String,
        _query: Vec<(String, String)>,
        _person: Option<String>,
    ) -> Result<String, String> {
        Err("no account".into())
    }
    fn on_webhook(
        _request: niles::integration::inbound::InboundRequest,
    ) -> niles::integration::inbound::InboundResponse {
        niles::integration::inbound::InboundResponse {
            status: 404,
            content_type: None,
            body: Vec::new(),
        }
    }
}

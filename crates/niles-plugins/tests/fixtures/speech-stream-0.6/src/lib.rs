wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::speech_stream;
use serde_json::{Value, json};

struct SpeechFixture;
export!(SpeechFixture);

impl Guest for SpeechFixture {
    fn init() -> Result<(), String> { Ok(()) }

    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "journey".into(),
            description: "Run the speech stream open, submit, poll, close journey.".into(),
            schema: json!({"type":"object","properties":{"room":{"type":"string"},"text":{"type":"string"}}}).to_string(),
        }]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        if name != "journey" { return Err(format!("unknown tool {name}")); }
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        let room = args.get("room").and_then(Value::as_str).unwrap_or("office");
        let text = args.get("text").and_then(Value::as_str).unwrap_or("Hello from the speech fixture.");
        let session = speech_stream::open(room).map_err(|e| format!("open: {e:?}"))?;
        let request = speech_stream::submit(session, text).map_err(|e| format!("submit: {e:?}"))?;
        let statuses = speech_stream::poll(session).map_err(|e| format!("poll: {e:?}"))?;
        speech_stream::close(session);
        let outcome = statuses.iter().find(|status| status.request == request)
            .map(|status| format!("{:?}", status.outcome))
            .unwrap_or_else(|| "Pending".into());
        Ok(json!({"session":session,"request":request,"outcome":outcome}).to_string())
    }

    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> { Ok(()) }
    fn on_timer(_id: String) {}
    fn on_event(_event: Event) {}
    fn render(_surface_id: String, _context: RenderContext) -> Result<UiTree, String> { Err("no UI".into()) }
    fn on_action(_surface_id: String, _control_id: String, _value: ActionValue, _context: RenderContext) -> Result<ActionResult, String> { Err("no UI".into()) }
    fn stop() {}
    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}
    fn on_mqtt_status(_conn: u32, _status: niles::integration::mqtt::ConnectionStatus) {}
    fn on_websocket_message(_conn: u32, _message: niles::integration::websocket::Message) {}
    fn on_websocket_status(_conn: u32, _status: niles::integration::websocket::ConnectionStatus) {}
    fn authorize_url(_callback_url: String, _state: String) -> Result<String, String> { Err("no account".into()) }
    fn on_callback(_callback_url: String, _query: Vec<(String, String)>, _person: Option<String>) -> Result<String, String> { Err("no account".into()) }
    fn on_webhook(_request: niles::integration::inbound::InboundRequest) -> niles::integration::inbound::InboundResponse {
        niles::integration::inbound::InboundResponse { status: 404, content_type: None, body: Vec::new() }
    }
}
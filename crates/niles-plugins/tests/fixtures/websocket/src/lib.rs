use niles::integration::websocket;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};

wit_bindgen::generate!({ world: "plugin", path: "../../../../../schemas/niles-integration.wit" });
struct Fixture;
export!(Fixture);
static CONN: AtomicU32 = AtomicU32::new(0);
static CONNECTED: AtomicU32 = AtomicU32::new(0);
static RECEIVED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);
static LAST_FAILURE: AtomicU32 = AtomicU32::new(0);

impl Guest for Fixture {
    fn init() -> Result<(), String> {
        let Some(niles::integration::config::SettingValue::Text(url)) =
            niles::integration::config::get("url")
        else {
            return Err("Missing URL".into());
        };
        let conn = websocket::connect("local", &url, &[]).map_err(|e| format!("{e:?}"))?;
        CONN.store(conn, Ordering::SeqCst);
        Ok(())
    }
    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "state".into(),
            description: "Observed WebSocket state".into(),
            schema: json!({"type":"object","properties":{}}).to_string(),
        }]
    }
    fn call_tool(name: String, _args_json: String) -> Result<String, String> {
        if name != "state" {
            return Err("unknown".into());
        }
        if _args_json == "{\"check_cap\":true}" {
            let Some(niles::integration::config::SettingValue::Text(url)) =
                niles::integration::config::get("url")
            else {
                return Err("Missing URL".into());
            };
            let mut extra = Vec::new();
            for _ in 0..3 {
                extra.push(websocket::connect("local", &url, &[]).map_err(|e| format!("{e:?}"))?);
            }
            if !matches!(
                websocket::connect("local", &url, &[]),
                Err(niles::integration::types::HostError::Limit(_))
            ) {
                return Err("fifth connection was not limited".into());
            }
            websocket::close(extra.pop().unwrap());
            extra.push(
                websocket::connect("local", &url, &[])
                    .map_err(|e| format!("slot not freed: {e:?}"))?,
            );
            for conn in extra {
                websocket::close(conn);
            }
            return Ok("cap verified".into());
        }
        Ok(json!({"conn":CONN.load(Ordering::SeqCst),"connected":CONNECTED.load(Ordering::SeqCst),"received":RECEIVED.load(Ordering::SeqCst),"failed":FAILED.load(Ordering::SeqCst),"last_failure":LAST_FAILURE.load(Ordering::SeqCst)}).to_string())
    }
    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Err("none".into())
    }
    fn on_timer(_id: String) {}
    fn on_event(_event: Event) {}
    fn render(_surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        Err("none".into())
    }
    fn on_action(
        _surface_id: String,
        _control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("none".into())
    }
    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}
    fn on_mqtt_status(_conn: u32, _status: ConnectionStatus) {}
    fn authorize_url(_callback_url: String, _state: String) -> Result<String, String> {
        Err("none".into())
    }
    fn on_callback(
        _callback_url: String,
        _query: Vec<(String, String)>,
        _person: Option<String>,
    ) -> Result<String, String> {
        Err("none".into())
    }
    fn on_webhook(_request: InboundRequest) -> InboundResponse {
        InboundResponse {
            status: 404,
            content_type: None,
            body: vec![],
        }
    }
    fn on_websocket_message(conn: u32, message: websocket::Message) {
        if conn == CONN.load(Ordering::SeqCst) {
            if matches!(&message, websocket::Message::Text(text) if text == "reconnect-now") {
                websocket::close(conn);
                return;
            }
            if matches!(&message, websocket::Message::Text(text) if text == "close-now") {
                websocket::close(conn);
                return;
            }
            RECEIVED.fetch_add(1, Ordering::SeqCst);
            let reply = match message {
                websocket::Message::Text(s) => websocket::Message::Text(format!("echo:{s}")),
                websocket::Message::Binary(b) => websocket::Message::Binary(b),
            };
            let _ = websocket::send(conn, &reply);
        }
    }
    fn on_websocket_status(conn: u32, status: websocket::ConnectionStatus) {
        if conn == CONN.load(Ordering::SeqCst) {
            if let websocket::ConnectionStatus::Failed(kind) = status {
                FAILED.fetch_add(1, Ordering::SeqCst);
                LAST_FAILURE.store(match kind {
                    websocket::FailureKind::Authentication => 1,
                    websocket::FailureKind::Tls => 2,
                    websocket::FailureKind::Policy => 3,
                    websocket::FailureKind::Protocol => 4,
                }, Ordering::SeqCst);
                return;
            }
        }
        if matches!(status, websocket::ConnectionStatus::Connected)
            && conn == CONN.load(Ordering::SeqCst)
        {
            CONNECTED.fetch_add(1, Ordering::SeqCst);
            let _ = websocket::send(conn, &websocket::Message::Text("from-component".into()));
        }
    }
    fn stop() {}
}

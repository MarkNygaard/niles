//! Interface versions side by side: `fixtures/relay.wasm`, built against
//! 0.2.0 and kept so, still loads and runs on a Niles that serves 0.2.1;
//! `fixtures/printer.wasm`, built against 0.2.1, uses what 0.2.1 added.

#![allow(clippy::unwrap_used, reason = "test helpers: a failure fails the test")]

use futures_util::{SinkExt, StreamExt};
use niles_core::{DeviceId, DeviceRegistry, Event, EventBus};
use niles_plugins::{House, Limits, MemoryStore, PluginStore, Runtime, Status};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

const PRINTER_MANIFEST: &str = include_str!("fixtures/printer/niles-integration.toml");
const PRINTER: &[u8] = include_bytes!("fixtures/printer.wasm");
const RELAY_MANIFEST: &str = include_str!("fixtures/relay/niles-integration.toml");
const RELAY: &[u8] = include_bytes!("fixtures/relay.wasm");
const WS_MANIFEST: &str = include_str!("fixtures/websocket/niles-integration.toml");
const WS_FIXTURE: &[u8] = include_bytes!("fixtures/websocket.wasm");

struct Home {
    runtime: Runtime,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    store: Arc<MemoryStore>,
}

impl Home {
    fn new(settings: toml::Table, store: Arc<MemoryStore>) -> Self {
        Self::build(settings, store, false)
    }

    fn websocket_home(settings: toml::Table, store: Arc<MemoryStore>) -> Self {
        Self::build(settings, store, true)
    }

    fn build(settings: toml::Table, store: Arc<MemoryStore>, loopback: bool) -> Self {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::new(1024);
        let house = House::new(registry.clone(), bus.clone(), store.clone())
            .with_settings(Arc::new(move |_| settings.clone()))
            .with_time_zone(Arc::new(|| "Europe/Copenhagen".to_string()));
        let house = if loopback {
            house.allowing_loopback()
        } else {
            house
        };
        Self {
            runtime: Runtime::new(house, Limits::default()).expect("the runtime starts"),
            registry,
            bus,
            store,
        }
    }

    async fn printer(store: Arc<MemoryStore>) -> Self {
        let home = Self::new(toml::Table::new(), store);
        home.runtime
            .install(PRINTER_MANIFEST, PRINTER.to_vec())
            .await
            .expect("the printer installs");
        home.running("printer").await;
        home
    }

    async fn running(&self, id: &str) {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let status = self.runtime.info(id).map(|i| i.status);
            if status == Some(Status::Running) {
                return;
            }
            assert!(
                Instant::now() < until,
                "{id} is {status:?}; log: {:?}",
                self.runtime.log(id)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn tool(&self, id: &str, name: &str, args: &str) -> String {
        self.runtime
            .call_tool(id, name, args)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn chamber_light(&self) -> niles_core::Device {
        self.registry
            .get(&DeviceId::parse("x_printer:office/chamber_light").unwrap())
            .expect("the chamber light is in the house")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_built_against_0_2_0_still_loads_and_runs() {
    let report = niles_plugins::check::check(RELAY_MANIFEST, Some(RELAY.to_vec()), None).await;
    assert!(report.ok(), "{:?}", report.problems);
    for import in ["devices", "config", "mqtt"] {
        let name = format!("niles:integration/{import}@0.2.0");
        assert!(
            report.imports.contains(&name),
            "relay.wasm is the 0.2.0 component this test is about; keep it built \
             against 0.2.0. It imports {:?}",
            report.imports
        );
    }

    // An address nothing answers on: the connection is tried in the
    // background, and the plugin runs meanwhile.
    let settings: toml::Table = toml::from_str("address = \"192.168.255.254\"").unwrap();
    let home = Home::new(settings, Arc::new(MemoryStore::new()));
    home.runtime
        .install(RELAY_MANIFEST, RELAY.to_vec())
        .await
        .expect("the 0.2.0 relay installs");
    home.running("relay").await;
    assert_eq!(home.tool("relay", "received", "{}").await, "[]");
    assert!(
        home.registry
            .get(&DeviceId::parse("x_relay:office/relay").unwrap())
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_0_2_1_component_is_told_the_house_time_zone() {
    let report = niles_plugins::check::check(PRINTER_MANIFEST, Some(PRINTER.to_vec()), None).await;
    assert!(report.ok(), "{:?}", report.problems);
    assert!(
        report
            .imports
            .contains(&"niles:integration/config@0.2.1".to_string())
    );

    let home = Home::printer(Arc::new(MemoryStore::new())).await;
    assert_eq!(
        home.tool("printer", "time_zone", "{}").await,
        "Europe/Copenhagen"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sensor_reports_an_action_and_is_remembered_as_one_that_does() {
    let store = Arc::new(MemoryStore::new());
    let home = Home::printer(store.clone()).await;
    let printer = DeviceId::parse("x_printer:office/printer").unwrap();
    assert!(!home.registry.get(&printer).unwrap().has_actions());

    let mut rx = home.bus.subscribe();
    assert_eq!(home.tool("printer", "finish", "{}").await, "finished");
    let action = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::DeviceAction { id, action } = rx.recv().await.unwrap() {
                return (id, action);
            }
        }
    })
    .await
    .expect("the action is on the bus");
    assert_eq!(action, (printer.clone(), "finished".to_string()));
    assert!(home.registry.get(&printer).unwrap().has_actions());

    let known = store.devices("printer").await.unwrap();
    assert!(
        known
            .iter()
            .any(|d| d.id == "office/printer" && d.reports_actions),
        "{known:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_light_can_keep_out_of_its_rooms_lights_and_is_remembered_so() {
    let store = Arc::new(MemoryStore::new());
    let home = Home::printer(store.clone()).await;
    let light = home.chamber_light();
    assert!(!light.room_light);
    assert!(!light.is_lamp(&Default::default()), "not one of the lights");
    assert!(light.is_switchable(), "still switched by name");

    assert_eq!(
        home.tool("printer", "room_light", r#"{"on":true}"#).await,
        "true"
    );
    assert!(home.chamber_light().room_light);
    assert_eq!(
        home.tool("printer", "room_light", r#"{"on":false}"#).await,
        "false"
    );

    // Started again over the same store: the light is back, kept out,
    // before its plugin has said anything.
    let again = Home::new(toml::Table::new(), store.clone());
    again
        .runtime
        .install(PRINTER_MANIFEST, PRINTER.to_vec())
        .await
        .expect("the printer installs again");
    assert!(!again.chamber_light().room_light);
    let known = home.store.devices("printer").await.unwrap();
    assert!(
        known
            .iter()
            .any(|d| d.id == "office/chamber_light" && !d.room_light),
        "{known:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_websocket_round_trip() {
    websocket_journey(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_connection_limit_and_slot_reuse() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let settings = toml::Table::from_iter([
        ("address".into(), toml::Value::String("127.0.0.1".into())),
        (
            "url".into(),
            toml::Value::String(format!("ws://127.0.0.1:{port}/")),
        ),
    ]);
    let home = Home::websocket_home(settings, Arc::new(MemoryStore::new()));
    let manifest = WS_MANIFEST.replace("port = 8088", &format!("port = {port}"));
    home.runtime
        .install(&manifest, WS_FIXTURE.to_vec())
        .await
        .unwrap();
    home.running("wsfixture").await;
    assert_eq!(
        home.tool("wsfixture", "state", "{\"check_cap\":true}")
            .await,
        "cap verified"
    );
    home.runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_http_401_is_terminal_without_retry() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let home = websocket_fixture_home(port).await;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        stream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err(),
            "401 must not retry"
        );
    });
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&home.tool("wsfixture", "state", "{}").await).unwrap();
    assert_eq!(state["failed"], 1);
    assert_eq!(state["last_failure"], 1);
    assert_eq!(state["connected"], 0);
    home.runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_http_503_retries_then_upgrades_and_echoes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let home = websocket_fixture_home(port).await;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await.unwrap();
        stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        let (stream, _) = tokio::time::timeout(Duration::from_secs(4), listener.accept())
            .await
            .expect("503 retry arrives")
            .unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap().into_text().unwrap(),
            "from-component"
        );
        ws.send(WsMessage::Text("from-server".into()))
            .await
            .unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap().into_text().unwrap(),
            "echo:from-server"
        );
        ws.close(None).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(7), server)
        .await
        .expect("503 retry/echo finishes")
        .unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&home.tool("wsfixture", "state", "{}").await).unwrap();
    assert_eq!(state["connected"], 1);
    assert_eq!(state["received"], 1);
    assert_eq!(state["failed"], 0);
    home.runtime.shutdown().await;
}

async fn websocket_fixture_home(port: u16) -> Home {
    let settings = toml::Table::from_iter([
        ("address".into(), toml::Value::String("127.0.0.1".into())),
        (
            "url".into(),
            toml::Value::String(format!("ws://127.0.0.1:{port}/")),
        ),
    ]);
    let home = Home::websocket_home(settings, Arc::new(MemoryStore::new()));
    let manifest = WS_MANIFEST.replace("port = 8088", &format!("port = {port}"));
    home.runtime
        .install(&manifest, WS_FIXTURE.to_vec())
        .await
        .unwrap();
    home.running("wsfixture").await;
    home
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_rejects_oversized_binary() {
    websocket_journey(Some(WsMessage::Binary(vec![0x5a; 64 * 1024 + 1].into()))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interface_0_4_rejects_oversized_text() {
    websocket_journey(Some(WsMessage::Text("x".repeat(64 * 1024 + 1).into()))).await;
}

async fn websocket_journey(oversized: Option<WsMessage>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        #[allow(clippy::result_large_err)]
        let request = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.uri().path(), "/socket/deep");
                assert_eq!(request.uri().query(), Some("token=not-logged&x=1"));
                Ok::<_, tokio_tungstenite::tungstenite::http::Response<Option<String>>>(response)
            },
        )
        .await
        .unwrap();
        let mut ws = request;
        let first = ws.next().await.unwrap().unwrap();
        assert_eq!(first.into_text().unwrap(), "from-component");
        ws.send(WsMessage::Text("from-server".into()))
            .await
            .unwrap();
        let reply = ws.next().await.unwrap().unwrap();
        assert_eq!(reply.into_text().unwrap(), "echo:from-server");
        if let Some(frame) = oversized {
            ws.send(frame).await.unwrap();
            assert!(
                matches!(
                    ws.next().await,
                    None | Some(Err(_)) | Some(Ok(WsMessage::Close(_)))
                ),
                "host must terminate the oversized stream, not echo it"
            );
        } else {
            ws.close(None).await.unwrap();
        }
        drop(ws);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap().into_text().unwrap(),
            "from-component"
        );
        ws.send(WsMessage::Text("after-reconnect".into()))
            .await
            .unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap().into_text().unwrap(),
            "echo:after-reconnect"
        );
        ws.send(WsMessage::Text("close-now".into())).await.unwrap();
        assert!(matches!(ws.next().await, Some(Ok(WsMessage::Close(_)))));
        assert!(
            tokio::time::timeout(Duration::from_millis(1500), listener.accept())
                .await
                .is_err(),
            "explicit close must not reconnect"
        );
    });
    let report = niles_plugins::check::check(WS_MANIFEST, Some(WS_FIXTURE.to_vec()), None).await;
    assert!(report.ok(), "{:?}", report.problems);
    let settings = toml::Table::from_iter([
        ("address".into(), toml::Value::String("127.0.0.1".into())),
        (
            "url".into(),
            toml::Value::String(format!(
                "ws://127.0.0.1:{port}/socket/deep?token=not-logged&x=1"
            )),
        ),
    ]);
    let home = Home::websocket_home(settings, Arc::new(MemoryStore::new()));
    let mut manifest = WS_MANIFEST.to_string();
    manifest = manifest.replace("port = 8088", &format!("port = {port}"));
    home.runtime
        .install(&manifest, WS_FIXTURE.to_vec())
        .await
        .expect("v0.4 installs");
    home.running("wsfixture").await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the WebSocket server completes the oversized-frame reconnect journey")
        .expect("the WebSocket server task succeeds");
    let state: serde_json::Value =
        serde_json::from_str(&home.tool("wsfixture", "state", "{}").await).unwrap();
    assert_eq!(state["connected"], 2);
    assert_eq!(state["received"], 2);
}

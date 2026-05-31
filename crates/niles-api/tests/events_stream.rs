use async_trait::async_trait;
use futures_util::StreamExt;
use niles_api::{AppState, router};
use niles_core::{
    Device, DeviceClass, DeviceId, DeviceName, DeviceRegistry, DeviceState, Event, EventBus,
    RoomName,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

#[derive(Default, Clone)]
struct MockPublisher;

#[async_trait]
impl niles_api::DevicePublisher for MockPublisher {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

fn make_state(bus: EventBus) -> AppState {
    AppState::new(
        Arc::new(DeviceRegistry::new()),
        Arc::new(MockPublisher),
        Arc::new("zigbee2mqtt".into()),
        bus,
    )
}

async fn spawn_server(bus: EventBus) -> u16 {
    let state = make_state(bus);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    // Give the server a moment to start listening.
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn recv_text(ws: &mut WsStream) -> String {
    let timeout = Duration::from_secs(2);
    loop {
        let msg = tokio::time::timeout(timeout, ws.next())
            .await
            .expect("timed out waiting for websocket message")
            .expect("websocket stream ended")
            .expect("websocket error");
        match msg {
            WsMessage::Text(t) => return t.to_string(),
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            other => panic!("expected Text, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn connect_receives_initial_ping() {
    let bus = EventBus::default();
    let port = spawn_server(bus).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let text = recv_text(&mut ws).await;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["type"], "ping");
    assert!(json["ts"].as_str().is_some());
}

#[tokio::test]
async fn device_state_changed_event_is_received() {
    let bus = EventBus::default();
    let port = spawn_server(bus.clone()).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Consume initial ping.
    let _ = recv_text(&mut ws).await;

    let id = DeviceId::parse("z2m:office/desk_lamp").unwrap();
    let state = DeviceState {
        on: Some(true),
        brightness: Some(80),
        ..Default::default()
    };
    bus.publish(Event::DeviceStateChanged {
        id: id.clone(),
        state: state.clone(),
    });

    let text = recv_text(&mut ws).await;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["type"], "device_state_changed");
    assert_eq!(json["id"], "z2m:office/desk_lamp");
    assert_eq!(json["state"]["on"], true);
    assert_eq!(json["state"]["brightness"], 80);
}

#[tokio::test]
async fn device_added_event_is_received() {
    let bus = EventBus::default();
    let port = spawn_server(bus.clone()).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Consume initial ping.
    let _ = recv_text(&mut ws).await;

    let id = DeviceId::parse("z2m:living_room/floor_lamp").unwrap();
    let device = Device::new(id.clone(), DeviceState::default(), DeviceClass::Light);
    bus.publish(Event::DeviceAdded { device });

    let text = recv_text(&mut ws).await;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["type"], "device_added");
    assert_eq!(json["id"], "z2m:living_room/floor_lamp");
    assert_eq!(json["class"], "light");
}

#[tokio::test]
async fn slow_consumer_gets_close_frame() {
    let bus = EventBus::new(10);
    let port = spawn_server(bus.clone()).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Don't read from ws. Fire 300 events.
    for i in 0..300 {
        let id = DeviceId::new(
            "z2m",
            RoomName::parse("room").unwrap(),
            DeviceName::parse(format!("device_{i}")).unwrap(),
        )
        .unwrap();
        bus.publish(Event::DeviceRemoved { id });
    }

    // Now read messages until we find the close frame.
    let mut found_close = false;
    for _ in 0..400 {
        let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let WsMessage::Text(t) = msg {
            let json: serde_json::Value = serde_json::from_str(&t).unwrap();
            if json["type"] == "close" {
                assert_eq!(json["reason"], "slow_consumer");
                assert!(json["dropped"].as_u64().unwrap() > 0);
                found_close = true;
                break;
            }
        }
    }
    assert!(found_close, "expected a close frame from slow consumer");
}

#[tokio::test]
async fn client_close_exits_cleanly() {
    let bus = EventBus::default();
    let port = spawn_server(bus).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    ws.close(None).await.unwrap();

    // Give the server a moment to process the close.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect to prove the server is still alive.
    let (mut ws2, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let text = recv_text(&mut ws2).await;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["type"], "ping");
}

#[tokio::test]
async fn two_connections_receive_same_event() {
    let bus = EventBus::default();
    let port = spawn_server(bus.clone()).await;
    let url = format!("ws://127.0.0.1:{port}/events/stream");
    let (mut ws1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (mut ws2, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Consume initial pings.
    let _ = recv_text(&mut ws1).await;
    let _ = recv_text(&mut ws2).await;

    let id = DeviceId::parse("z2m:office/desk_lamp").unwrap();
    bus.publish(Event::DeviceRemoved { id: id.clone() });

    let text1 = recv_text(&mut ws1).await;
    let text2 = recv_text(&mut ws2).await;

    let json1: serde_json::Value = serde_json::from_str(&text1).unwrap();
    let json2: serde_json::Value = serde_json::from_str(&text2).unwrap();
    assert_eq!(json1, json2);
    assert_eq!(json1["type"], "device_removed");
    assert_eq!(json1["id"], "z2m:office/desk_lamp");
}

#![allow(clippy::unwrap_used, reason = "test helpers: a failure fails the test")]

//! The live browser's socket over a real connection: pictures out, the
//! person's input in, and a page from elsewhere turned away.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use niles_api::browser::{Go, LiveBrowser, LiveView, Refused, Then, Watching};
use niles_api::{AppState, router};
use niles_core::{DeviceRegistry, EventBus};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

#[derive(Default, Clone)]
struct MockPublisher;

#[async_trait]
impl niles_api::DevicePublisher for MockPublisher {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

/// Two pictures, then nothing more until the socket goes.
struct Pictures(Vec<Arc<[u8]>>);

#[async_trait]
impl Watching for Pictures {
    async fn next(&mut self) -> Option<Arc<[u8]>> {
        if self.0.is_empty() {
            std::future::pending::<()>().await;
        }
        Some(self.0.remove(0))
    }
}

/// A browser that is open unless `closed`, and keeps the input it gets.
#[derive(Default)]
struct Fake {
    closed: bool,
    inputs: Mutex<Vec<String>>,
}

#[async_trait]
impl LiveBrowser for Fake {
    async fn view(&self, _who: &str, _speaker: Option<&str>) -> Option<LiveView> {
        None
    }
    async fn take(&self, _who: &str, _speaker: Option<&str>) -> Result<(), Refused> {
        Ok(())
    }
    async fn give_back(
        &self,
        _who: &str,
        _speaker: Option<&str>,
        _conversation: Option<i64>,
        _then: Then,
    ) -> Result<(), Refused> {
        Ok(())
    }
    async fn go(&self, _who: &str, _speaker: Option<&str>, _go: Go) -> Result<(), Refused> {
        Ok(())
    }
    async fn input(&self, _who: &str, _speaker: Option<&str>, input: &str) -> Result<(), Refused> {
        if input.contains("niles-has-it") {
            return Err(Refused::NotInControl);
        }
        self.inputs.lock().unwrap().push(input.to_string());
        Ok(())
    }
    async fn watch(&self, _who: &str, _speaker: Option<&str>) -> Option<Box<dyn Watching>> {
        if self.closed {
            return None;
        }
        Some(Box::new(Pictures(vec![
            Arc::from(&[0xFF, 0xD8, 1][..]),
            Arc::from(&[0xFF, 0xD8, 2][..]),
        ])))
    }
}

async fn spawn_server(fake: Arc<Fake>) -> u16 {
    let state = AppState::new(
        Arc::new(DeviceRegistry::new()),
        Arc::new(MockPublisher),
        Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
        EventBus::default(),
    )
    .with_live_browser(Some(fake as Arc<dyn LiveBrowser>));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

async fn next_message(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsMessage {
    tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("a message within two seconds")
        .expect("the socket stays open")
        .expect("no socket error")
}

#[tokio::test]
async fn pictures_go_out_and_the_persons_input_comes_in() {
    let fake = Arc::new(Fake::default());
    let port = spawn_server(fake.clone()).await;
    let url = format!("ws://127.0.0.1:{port}/me/browser/live");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    for n in [1u8, 2] {
        match next_message(&mut ws).await {
            WsMessage::Binary(bytes) => assert_eq!(&bytes[..], &[0xFF, 0xD8, n]),
            other => panic!("expected a picture, got {other:?}"),
        }
    }

    let click = r#"{"type":"press","x":10,"y":20,"press":"down","button":"left"}"#;
    ws.send(WsMessage::Text(click.into())).await.unwrap();
    ws.send(WsMessage::Text("niles-has-it".into()))
        .await
        .unwrap();
    match next_message(&mut ws).await {
        WsMessage::Text(said) => assert_eq!(said.as_str(), r#"{"refused":"not_in_control"}"#),
        other => panic!("expected the refusal, got {other:?}"),
    }
    assert_eq!(*fake.inputs.lock().unwrap(), [click.to_string()]);
}

#[tokio::test]
async fn a_page_from_elsewhere_is_turned_away() {
    let port = spawn_server(Arc::new(Fake::default())).await;
    let mut request = format!("ws://127.0.0.1:{port}/me/browser/live")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let refused = tokio_tungstenite::connect_async(request).await;
    match refused {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 403);
        }
        other => panic!("expected a 403, got {other:?}"),
    }
}

#[tokio::test]
async fn nothing_open_is_no_socket() {
    let port = spawn_server(Arc::new(Fake {
        closed: true,
        ..Fake::default()
    }))
    .await;
    let refused =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/me/browser/live")).await;
    match refused {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 404);
        }
        other => panic!("expected a 404, got {other:?}"),
    }
}

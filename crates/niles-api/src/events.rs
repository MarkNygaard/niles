//! WebSocket event stream handler.

use crate::dto::{DeviceClassDto, DeviceStateDto};
use crate::state::AppState;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use niles_core::{Event, EventBus};
use serde::Serialize;
use std::time::Duration;
use tokio::select;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Maximum number of events queued per connection before dropping
/// the client as a slow consumer.
const BROADCAST_BUFFER: usize = 256;

/// Application-level heartbeat interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Wire format for events sent over the WebSocket.
///
/// Uses `type` as the discriminator. Only a subset of `niles_core::Event`
/// variants are forwarded; others are silently dropped.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WireEvent {
    DeviceStateChanged {
        id: String,
        state: DeviceStateDto,
    },
    DeviceAdded {
        id: String,
        #[serde(rename = "class")]
        class: DeviceClassDto,
    },
    DeviceRemoved {
        id: String,
    },
    Ping {
        ts: String,
    },
    Close {
        reason: String,
        dropped: u64,
    },
}

impl WireEvent {
    fn from_event(event: Event) -> Option<Self> {
        match event {
            Event::DeviceStateChanged { id, state } => Some(WireEvent::DeviceStateChanged {
                id: id.to_string(),
                state: (&state).into(),
            }),
            Event::DeviceAdded { device } => Some(WireEvent::DeviceAdded {
                id: device.id.to_string(),
                class: (&device.class).into(),
            }),
            Event::DeviceRemoved { id } => Some(WireEvent::DeviceRemoved { id: id.to_string() }),
            _ => None,
        }
    }
}

/// # Event stream
///
/// Upgrades the HTTP connection to a WebSocket and streams internal
/// bus events as JSON text frames.
///
/// Supports:
/// - `device_state_changed`, `device_added`, `device_removed` events
/// - A 30-second application-level heartbeat (`{"type":"ping"}`)
/// - Slow-consumer detection: if >256 events queue up, the connection
///   is closed with `{"type":"close","reason":"slow_consumer"}`
pub async fn events_stream(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state.event_bus))
}

async fn handle_socket(mut socket: WebSocket, bus: EventBus) {
    let mut rx = bus.subscribe();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut dropped: u64 = 0;

    let close_reason = loop {
        select! {
            _ = heartbeat.tick() => {
                let ping = WireEvent::Ping {
                    ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                };
                if send_json(&mut socket, &ping).await.is_err() {
                    break "send_error";
                }
            }
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Some(wire) = WireEvent::from_event(event)
                            && send_json(&mut socket, &wire).await.is_err()
                        {
                            break "send_error";
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        dropped += n;
                        if dropped > BROADCAST_BUFFER as u64 {
                            let close = WireEvent::Close {
                                reason: "slow_consumer".to_string(),
                                dropped,
                            };
                            let _ = send_json(&mut socket, &close).await;
                            break "slow_consumer";
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break "bus_closed",
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) => break "client_close",
                    None => break "client_disconnect",
                    Some(Ok(_)) => {} // ignore other client messages
                    Some(Err(_)) => break "protocol_error",
                }
            }
        }
    };

    if close_reason == "slow_consumer" {
        warn!("WebSocket event stream closed: {close_reason} ({dropped} events dropped)");
    } else {
        debug!("WebSocket event stream closed: {close_reason}");
    }
}

async fn send_json(socket: &mut WebSocket, event: &WireEvent) -> Result<(), ()> {
    let json = serde_json::to_string(event).map_err(|e| {
        warn!("Failed to serialize WireEvent: {e}");
    })?;
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{Device, DeviceClass, DeviceId, DeviceState};

    #[test]
    fn ping_serializes() {
        let ping = WireEvent::Ping {
            ts: "2026-05-30T14:30:00Z".to_string(),
        };
        let v = serde_json::to_value(&ping).unwrap();
        assert_eq!(v["type"], "ping");
        assert_eq!(v["ts"], "2026-05-30T14:30:00Z");
    }

    #[test]
    fn close_serializes() {
        let close = WireEvent::Close {
            reason: "slow_consumer".to_string(),
            dropped: 42,
        };
        let v = serde_json::to_value(&close).unwrap();
        assert_eq!(v["type"], "close");
        assert_eq!(v["reason"], "slow_consumer");
        assert_eq!(v["dropped"], 42);
    }

    #[test]
    fn device_state_changed_serializes() {
        let id = DeviceId::parse("z2m:office/desk_lamp").unwrap();
        let state = DeviceState {
            on: Some(true),
            brightness: Some(80),
            ..Default::default()
        };
        let ev = WireEvent::DeviceStateChanged {
            id: id.to_string(),
            state: (&state).into(),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "device_state_changed");
        assert_eq!(v["id"], "z2m:office/desk_lamp");
        assert_eq!(v["state"]["on"], true);
        assert_eq!(v["state"]["brightness"], 80);
    }

    #[test]
    fn device_added_serializes() {
        let id = DeviceId::parse("z2m:living_room/floor_lamp").unwrap();
        let device = Device::new(id.clone(), DeviceState::default(), DeviceClass::Light);
        let ev = WireEvent::DeviceAdded {
            id: device.id.to_string(),
            class: (&device.class).into(),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "device_added");
        assert_eq!(v["id"], "z2m:living_room/floor_lamp");
        assert_eq!(v["class"], "light");
    }

    #[test]
    fn device_removed_serializes() {
        let id = DeviceId::parse("z2m:kitchen/old_outlet").unwrap();
        let ev = WireEvent::DeviceRemoved { id: id.to_string() };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "device_removed");
        assert_eq!(v["id"], "z2m:kitchen/old_outlet");
    }

    #[test]
    fn unsupported_event_returns_none() {
        let id = DeviceId::parse("z2m:kitchen/button").unwrap();
        let ev = Event::DeviceAction {
            id,
            action: "on_press".to_string(),
        };
        assert!(WireEvent::from_event(ev).is_none());
    }
}

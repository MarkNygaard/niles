//! Axum router + serve entry point.

use crate::handlers;
use crate::state::AppState;
use axum::Router;
use axum::routing::get;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

/// Build the API router with the given shared state. Exposed so
/// tests can drive it via `tower::ServiceExt::oneshot` without
/// binding a port.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/devices", get(handlers::list_devices))
        .route("/rooms/{room}", get(handlers::devices_in_room))
        .with_state(state)
}

/// Bind to `addr` and run the API server until the process exits
/// or the listener fails.
pub async fn serve(addr: SocketAddr, state: AppState) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!("niles-api listening on http://{addr}");
    axum::serve(listener, router(state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use niles_core::{Device, DeviceId, DeviceName, DeviceRegistry, DeviceState, RoomName};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn make_state() -> AppState {
        AppState::new(Arc::new(DeviceRegistry::new()))
    }

    fn make_device(room: &str, name: &str) -> Device {
        Device {
            id: DeviceId::new(
                "z2m",
                RoomName::parse(room).unwrap(),
                DeviceName::parse(name).unwrap(),
            )
            .unwrap(),
            state: DeviceState::default(),
        }
    }

    async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = if body_bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body_bytes).unwrap_or(Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            ))
        };
        (status, body)
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let state = make_state();
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn list_devices_empty_returns_empty_array() {
        let state = make_state();
        let app = router(state);
        let (status, body) = get(app, "/devices").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!([]));
    }

    #[tokio::test]
    async fn list_devices_returns_registered_devices() {
        let state = make_state();
        state
            .registry
            .upsert(make_device("kitchen", "ceiling_light"));
        state.registry.upsert(make_device("office", "desk_lamp"));
        let app = router(state);

        let (status, body) = get(app, "/devices").await;
        assert_eq!(status, StatusCode::OK);
        let arr = body.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        let ids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"z2m:kitchen/ceiling_light"));
        assert!(ids.contains(&"z2m:office/desk_lamp"));
    }

    #[tokio::test]
    async fn devices_in_room_filters() {
        let state = make_state();
        state
            .registry
            .upsert(make_device("kitchen", "ceiling_light"));
        state
            .registry
            .upsert(make_device("kitchen", "counter_light"));
        state.registry.upsert(make_device("office", "desk_lamp"));
        let app = router(state);

        let (status, body) = get(app, "/rooms/kitchen").await;
        assert_eq!(status, StatusCode::OK);
        let arr = body.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for d in arr {
            assert_eq!(d["room"], "kitchen");
        }
    }

    #[tokio::test]
    async fn devices_in_room_rejects_invalid_room_name() {
        let state = make_state();
        let app = router(state);
        // Uppercase fails RoomName validation.
        let (status, _body) = get(app, "/rooms/Kitchen").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn device_dto_includes_flat_id_components() {
        let state = make_state();
        let mut device = make_device("kitchen", "ceiling_light");
        device.state.on = Some(true);
        device.state.brightness = Some(80);
        state.registry.upsert(device);
        let app = router(state);

        let (status, body) = get(app, "/devices").await;
        assert_eq!(status, StatusCode::OK);
        let d = &body.as_array().unwrap()[0];
        assert_eq!(d["id"], "z2m:kitchen/ceiling_light");
        assert_eq!(d["source"], "z2m");
        assert_eq!(d["room"], "kitchen");
        assert_eq!(d["name"], "ceiling_light");
        assert_eq!(d["state"]["on"], true);
        assert_eq!(d["state"]["brightness"], 80);
        // Unset fields are JSON null:
        assert_eq!(d["state"]["color_temp_kelvin"], Value::Null);
    }
}

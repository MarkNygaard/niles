//! Axum router + serve entry point.

use crate::handlers;
use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post};
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
        .route("/rooms/{room}/{device}", post(handlers::set_device))
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
    use crate::publish::DevicePublisher;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use niles_core::{
        Device, DeviceClass, DeviceId, DeviceName, DeviceRegistry, DeviceState, RoomName,
    };
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    #[derive(Default, Clone)]
    #[allow(clippy::type_complexity)]
    struct MockPublisher {
        sent: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    }

    impl MockPublisher {
        fn calls(&self) -> Vec<(String, Vec<u8>)> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl DevicePublisher for MockPublisher {
        async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
            self.sent.lock().unwrap().push((topic, payload));
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FailingPublisher;

    #[async_trait]
    impl DevicePublisher for FailingPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Err("broker unreachable".into())
        }
    }

    fn make_state() -> AppState {
        AppState::new(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
            Arc::new("zigbee2mqtt".into()),
        )
    }

    fn make_device(room: &str, name: &str) -> Device {
        Device::new(
            DeviceId::new(
                "z2m",
                RoomName::parse(room).unwrap(),
                DeviceName::parse(name).unwrap(),
            )
            .unwrap(),
            DeviceState::default(),
            DeviceClass::Unknown,
        )
    }

    fn make_light(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Light;
        d.state.on = Some(true);
        d.state.brightness = Some(100);
        d
    }

    fn make_sensor(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Sensor;
        d
    }

    fn make_switch(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Switch;
        d
    }

    async fn decode_response(response: axum::response::Response) -> (StatusCode, Value) {
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

    async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        decode_response(response).await
    }

    async fn post(app: Router, uri: &str, json_body: Value) -> (StatusCode, Value) {
        let body = serde_json::to_vec(&json_body).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        decode_response(response).await
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
        device.class = DeviceClass::Light;
        state.registry.upsert(device);
        let app = router(state);

        let (status, body) = get(app, "/devices").await;
        assert_eq!(status, StatusCode::OK);
        let d = &body.as_array().unwrap()[0];
        assert_eq!(d["id"], "z2m:kitchen/ceiling_light");
        assert_eq!(d["source"], "z2m");
        assert_eq!(d["room"], "kitchen");
        assert_eq!(d["name"], "ceiling_light");
        assert_eq!(d["class"], "light");
        assert_eq!(d["state"]["on"], true);
        assert_eq!(d["state"]["brightness"], 80);
        // Unset fields are JSON null:
        assert_eq!(d["state"]["color_temp_kelvin"], Value::Null);
    }

    #[tokio::test]
    async fn device_dto_serializes_all_class_variants() {
        let state = make_state();
        for (room, name, class) in [
            ("kitchen", "light", DeviceClass::Light),
            ("kitchen", "dimmer", DeviceClass::Switch),
            ("kitchen", "thermometer", DeviceClass::Sensor),
            ("kitchen", "mystery", DeviceClass::Unknown),
        ] {
            let mut d = make_device(room, name);
            d.class = class;
            state.registry.upsert(d);
        }
        let app = router(state);

        let (status, body) = get(app, "/devices").await;
        assert_eq!(status, StatusCode::OK);
        let by_name: std::collections::HashMap<&str, &str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["name"].as_str().unwrap(), d["class"].as_str().unwrap()))
            .collect();
        assert_eq!(by_name["light"], "light");
        assert_eq!(by_name["dimmer"], "switch");
        assert_eq!(by_name["thermometer"], "sensor");
        assert_eq!(by_name["mystery"], "unknown");
    }

    // ---- POST /rooms/{room}/{device} tests ----

    #[tokio::test]
    async fn post_set_on_to_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "zigbee2mqtt/office/desk_lamp/set");
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"state\":\"ON\""), "payload: {payload}");
    }

    #[tokio::test]
    async fn post_set_off_to_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": false}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"state\":\"OFF\""), "payload: {payload}");
    }

    #[tokio::test]
    async fn post_set_multiple_fields_to_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true, "brightness": 50}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"state\":\"ON\""), "payload: {payload}");
        assert!(payload.contains("\"brightness\":127"), "payload: {payload}");
    }

    #[tokio::test]
    async fn post_set_brightness_to_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"brightness": 50}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"brightness\":127"), "payload: {payload}");
    }

    #[tokio::test]
    async fn post_set_color_temp_to_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"color_temp_kelvin": 4000}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"color_temp\":250"), "payload: {payload}");
    }

    #[tokio::test]
    async fn post_to_missing_device_returns_not_found() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_to_sensor_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_sensor("kitchen", "thermometer"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/kitchen/thermometer",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_to_switch_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_switch("hallway", "dimmer"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/hallway/dimmer",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_empty_body_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(app, "/rooms/office/desk_lamp", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_brightness_at_boundary_is_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        // 0 and 100 are the inclusive bounds.
        for &pct in [0, 100].iter() {
            let (status, _body) = post(
                app.clone(),
                "/rooms/office/desk_lamp",
                serde_json::json!({"brightness": pct}),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::ACCEPTED,
                "brightness {pct} should be accepted"
            );
        }
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn post_brightness_out_of_range_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"brightness": 150}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_kelvin_at_boundary_is_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        // 1000 and 10000 are the inclusive bounds.
        for &k in [1000, 10000].iter() {
            let (status, _body) = post(
                app.clone(),
                "/rooms/office/desk_lamp",
                serde_json::json!({"color_temp_kelvin": k}),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::ACCEPTED,
                "color_temp_kelvin {k} should be accepted"
            );
        }
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn post_kelvin_out_of_range_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"color_temp_kelvin": 50000}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_invalid_room_name_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/OFFICE/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_invalid_device_name_returns_bad_request() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        // Hyphen is rejected by DeviceName validation.
        let (status, _body) = post(
            app,
            "/rooms/office/desk-lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_unknown_field_returns_unprocessable() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(registry, mock.clone(), Arc::new("zigbee2mqtt".into()));
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true, "unknown_field": "value"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_failing_publisher_returns_bad_gateway() {
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let state = AppState::new(
            registry,
            Arc::new(FailingPublisher),
            Arc::new("zigbee2mqtt".into()),
        );
        let app = router(state);

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}

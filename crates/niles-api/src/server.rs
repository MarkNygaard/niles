//! Axum router + serve entry point.

use crate::handlers;
use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post, put};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

/// Build the API router with the given shared state. Exposed so
/// tests can drive it via `tower::ServiceExt::oneshot` without
/// binding a port.
pub fn router(state: AppState) -> Router {
    let mut r = Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/devices", get(handlers::list_devices))
        .route("/lights", post(handlers::set_all_lights))
        .route("/logs", get(crate::logs::get_logs))
        .route(
            "/rooms/{room}",
            get(handlers::devices_in_room).post(handlers::set_room),
        )
        .route("/rooms/{room}/{device}", post(handlers::set_device))
        .route("/events/stream", get(crate::events::events_stream))
        .route("/setup", get(crate::presence::setup_report))
        .route("/secrets", get(crate::secrets::list_secrets))
        .route(
            "/secrets/{key}",
            put(crate::secrets::set_secret).delete(crate::secrets::clear_secret),
        )
        .route("/presence/tado", get(crate::presence::tado_status))
        .route(
            "/presence/tado/connect",
            post(crate::presence::tado_connect),
        )
        .route("/auth/status", get(crate::auth::status))
        .route("/auth/github/start", get(crate::auth::github::start))
        .route("/auth/github/callback", get(crate::auth::github::callback))
        .route("/auth/signout", get(crate::auth::github::sign_out))
        .route(
            "/config",
            get(crate::config::get_config).patch(crate::config::patch_config),
        )
        .route("/config/history", get(crate::config::get_history))
        .route("/config/undo", post(crate::config::undo_config))
        .route(
            "/config/{path}",
            axum::routing::delete(crate::config::reset_config),
        );
    if state.linear_webhook.is_some() {
        r = r.route("/webhooks/linear", post(crate::webhook::handle_linear));
    }
    // Registered last so every API route above wins; the UI takes what is
    // left over, including unknown paths (it routes them client-side).
    #[cfg(feature = "ui")]
    {
        r = r.fallback(crate::web::serve_asset);
    }
    // Wrapped around everything, including the fallback: a route added
    // later is behind the gate by default, and has to be named in
    // `auth::is_exempt` to get out from behind it.
    r.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::auth::require_sign_in,
    ))
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
        Device, DeviceClass, DeviceId, DeviceName, DeviceRegistry, DeviceState, EventBus,
        LightCapabilities, RoomName,
    };
    use niles_mqtt::CommandRouter;
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
            Arc::new(CommandRouter::z2m_only("zigbee2mqtt")),
            EventBus::default(),
        )
    }

    fn app_with(registry: Arc<DeviceRegistry>, publisher: Arc<dyn DevicePublisher>) -> Router {
        app_routing(registry, publisher, CommandRouter::z2m_only("zigbee2mqtt"))
    }

    fn app_routing(
        registry: Arc<DeviceRegistry>,
        publisher: Arc<dyn DevicePublisher>,
        command_router: CommandRouter,
    ) -> Router {
        router(AppState::new(
            registry,
            publisher,
            Arc::new(command_router),
            EventBus::default(),
        ))
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

    /// A bulb that can do everything — colour and a white channel — so
    /// tests exercising one of those aren't also asserting a capability.
    fn make_light(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Light;
        d.state.on = Some(true);
        d.state.brightness = Some(100);
        d.capabilities = LightCapabilities {
            rgb: true,
            color_temp: true,
        };
        d
    }

    /// A plain bulb: on, off, and a level. No colour of any kind.
    fn make_dimmable(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Light;
        d.state.on = Some(true);
        d
    }

    fn make_outlet(room: &str, name: &str) -> Device {
        let mut d = make_device(room, name);
        d.class = DeviceClass::Outlet;
        d.state.on = Some(true);
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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(app, "/rooms/office/desk_lamp", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn post_brightness_at_boundary_is_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, mock.clone());

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
    async fn post_to_a_name_the_room_does_not_have_returns_not_found() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(
            app,
            "/rooms/office/desk-lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(mock.calls().is_empty());
    }

    // ---- addressing a light across sources ----

    fn make_wled_light(room: &str, name: &str) -> Device {
        let mut d = Device::new(
            DeviceId::new(
                "wled",
                RoomName::parse(room).unwrap(),
                DeviceName::parse(name).unwrap(),
            )
            .unwrap(),
            DeviceState::default(),
            DeviceClass::Light,
        )
        .with_capabilities(LightCapabilities {
            rgb: true,
            color_temp: false,
        });
        d.state.on = Some(true);
        d
    }

    fn router_with_wled(room: &str, name: &str, topic: &str) -> CommandRouter {
        let mut map = std::collections::HashMap::new();
        map.insert(
            DeviceId::new(
                "wled",
                RoomName::parse(room).unwrap(),
                DeviceName::parse(name).unwrap(),
            )
            .unwrap(),
            topic.to_string(),
        );
        CommandRouter::new("zigbee2mqtt", map)
    }

    #[tokio::test]
    async fn a_wled_light_is_reachable_from_the_api() {
        // Before this the write route formatted every command as Z2M,
        // so a strip could be listed but never switched.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_wled_light("office", "desk_strip"));
        let app = app_routing(
            registry,
            mock.clone(),
            router_with_wled("office", "desk_strip", "wled/office"),
        );

        let (status, _body) = post(
            app,
            "/rooms/office/desk_strip",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(mock.calls().len(), 1);
        assert!(
            mock.calls()[0].0.starts_with("wled/office"),
            "should speak WLED, got {}",
            mock.calls()[0].0
        );
    }

    #[tokio::test]
    async fn one_name_in_two_sources_asks_which_rather_than_guessing() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "ceiling"));
        registry.upsert(make_wled_light("office", "ceiling"));
        let app = app_routing(
            registry,
            mock.clone(),
            router_with_wled("office", "ceiling", "wled/office"),
        );

        let (status, body) = post(
            app,
            "/rooms/office/ceiling",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body.as_str().unwrap_or_default().contains("wled"),
            "the error should name the sources: {body}"
        );
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn a_source_qualified_name_picks_that_source() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "ceiling"));
        registry.upsert(make_wled_light("office", "ceiling"));
        let app = app_routing(
            registry,
            mock.clone(),
            router_with_wled("office", "ceiling", "wled/office"),
        );

        let (status, _body) = post(
            app,
            "/rooms/office/wled:ceiling",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(mock.calls()[0].0.starts_with("wled/office"));
    }

    #[tokio::test]
    async fn post_rgb_to_a_light_returns_accepted() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"rgb": [255, 0, 0]}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let calls = mock.calls();
        let payload = std::str::from_utf8(&calls[0].1).unwrap();
        assert!(payload.contains("\"r\":255"), "payload: {payload}");
    }

    #[tokio::test]
    async fn a_colour_and_a_white_together_are_refused() {
        // The light can be in one mode or the other; sending both
        // leaves which one wins to the firmware.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"rgb": [255, 0, 0], "color_temp_kelvin": 4000}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    // ---- what a given device can be told ----

    #[tokio::test]
    async fn a_lamp_on_a_smart_plug_can_still_be_switched() {
        // It is a light to whoever owns it, and the dashboard lists it
        // as one; refusing "off" because Z2M calls it an outlet would
        // leave it the one lamp in the house the UI can't reach.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(
            app,
            "/rooms/living_room/corner_lamp",
            serde_json::json!({"on": false}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn an_outlet_refuses_a_brightness_rather_than_dropping_it() {
        // Accepting it and sending half the command would report
        // success for something that never happened.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(
            app,
            "/rooms/living_room/corner_lamp",
            serde_json::json!({"brightness": 40}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body.as_str().unwrap_or_default().contains("on or off"),
            "the error should say what it can do: {body}"
        );
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn a_bulb_with_no_colour_refuses_one() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_dimmable("office", "go"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(
            app,
            "/rooms/office/go",
            serde_json::json!({"rgb": [255, 0, 0]}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.as_str().unwrap_or_default().contains("colour channel"));
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn a_room_command_narrows_to_what_each_device_can_act_on() {
        // The caller named the room, not the devices, so a colour meant
        // for the strip is simply not sent to the plug or the plain
        // bulb — and they still hear the part they can act on.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("living_room", "strip"));
        registry.upsert(make_dimmable("living_room", "bulb"));
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(
            app,
            "/rooms/living_room",
            serde_json::json!({"on": true, "brightness": 60, "rgb": [255, 0, 0]}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["lights"], 3);

        let sent: Vec<(String, String)> = mock
            .calls()
            .into_iter()
            .map(|(t, p)| (t, String::from_utf8(p).unwrap()))
            .collect();
        let payload = |name: &str| {
            sent.iter()
                .find(|(t, _)| t.contains(name))
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| panic!("nothing sent to {name}: {sent:?}"))
        };

        assert!(
            payload("strip").contains("\"r\":255"),
            "the strip gets the colour"
        );
        assert!(
            !payload("bulb").contains("color"),
            "the plain bulb does not"
        );
        assert!(
            payload("bulb").contains("brightness"),
            "but it does get the level"
        );

        let lamp = payload("corner_lamp");
        assert!(
            lamp.contains("\"state\":\"ON\""),
            "the plug hears the switch"
        );
        assert!(
            !lamp.contains("brightness"),
            "and nothing it can't act on: {lamp}"
        );
    }

    #[tokio::test]
    async fn a_room_where_nothing_can_act_on_it_says_so() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(
            app,
            "/rooms/living_room",
            serde_json::json!({"brightness": 60}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn a_room_of_switches_and_sensors_is_not_a_room_of_lights() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_switch("all", "bedroom_switch"));
        registry.upsert(make_sensor("all", "thermometer"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(app, "/rooms/all", serde_json::json!({"on": true})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ---- POST /lights ----

    #[tokio::test]
    async fn posting_to_lights_reaches_the_whole_house() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("kitchen", "ceiling"));
        registry.upsert(make_light("office", "go"));
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(app, "/lights", serde_json::json!({"on": false})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["lights"], 3, "every room, not just one");
        assert_eq!(mock.calls().len(), 3);
    }

    #[tokio::test]
    async fn the_house_leaves_sensors_and_wall_switches_alone() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("kitchen", "ceiling"));
        registry.upsert(make_switch("all", "bedroom_switch"));
        registry.upsert(make_sensor("hallway", "thermometer"));
        let app = app_with(registry, mock.clone());

        let (_, body) = post(app, "/lights", serde_json::json!({"on": false})).await;
        assert_eq!(body["lights"], 1);
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn the_house_narrows_per_device_like_a_room_does() {
        // Same rule: the caller named the house, not a device, so a
        // colour goes only where it can be shown.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("living_room", "strip"));
        registry.upsert(make_outlet("living_room", "corner_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, _) = post(
            app,
            "/lights",
            serde_json::json!({"on": true, "rgb": [255, 0, 0]}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let sent: Vec<String> = mock
            .calls()
            .into_iter()
            .map(|(t, p)| format!("{t} {}", String::from_utf8(p).unwrap()))
            .collect();
        let lamp = sent
            .iter()
            .find(|line| line.contains("corner_lamp"))
            .expect("the plug still hears the switch");
        assert!(lamp.contains("\"state\":\"ON\""));
        assert!(
            !lamp.contains("color"),
            "and nothing it cannot act on: {lamp}"
        );
    }

    #[tokio::test]
    async fn a_house_with_no_lights_says_so() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_sensor("hallway", "thermometer"));
        let app = app_with(registry, mock.clone());

        let (status, _) = post(app, "/lights", serde_json::json!({"on": true})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn an_empty_body_is_refused_for_the_house_too() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("kitchen", "ceiling"));
        let app = app_with(registry, mock.clone());

        let (status, _) = post(app, "/lights", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(mock.calls().is_empty());
    }

    // ---- POST /rooms/{room} ----

    #[tokio::test]
    async fn posting_to_a_room_reaches_every_light_in_it() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("kitchen", "ceiling"));
        registry.upsert(make_light("kitchen", "counter"));
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(app, "/rooms/kitchen", serde_json::json!({"on": false})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["lights"], 2);

        let topics: Vec<String> = mock.calls().into_iter().map(|(t, _)| t).collect();
        assert_eq!(topics.len(), 2);
        assert!(topics.iter().all(|t| t.contains("/kitchen/")));
    }

    #[tokio::test]
    async fn posting_to_a_room_leaves_sensors_and_wall_switches_alone() {
        // A room's thermometer has no business receiving "off".
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("kitchen", "ceiling"));
        registry.upsert(make_sensor("kitchen", "thermometer"));
        registry.upsert(make_switch("kitchen", "dimmer"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(app, "/rooms/kitchen", serde_json::json!({"on": true})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["lights"], 1);
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn posting_to_a_room_with_no_lights_returns_not_found() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_sensor("hallway", "thermometer"));
        let app = app_with(registry, mock.clone());

        let (status, _body) = post(app, "/rooms/hallway", serde_json::json!({"on": true})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn a_light_with_no_route_does_not_stop_the_rest_of_the_room() {
        // A WLED strip with no `[wled]` entry can't be commanded. That
        // is a config gap, not a reason to leave the ceiling light on.
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "ceiling"));
        registry.upsert(make_wled_light("office", "unconfigured_strip"));
        let app = app_with(registry, mock.clone());

        let (status, body) = post(app, "/rooms/office", serde_json::json!({"on": false})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["lights"], 1);
    }

    #[tokio::test]
    async fn post_unknown_field_returns_unprocessable() {
        let mock = Arc::new(MockPublisher::default());
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "desk_lamp"));
        let app = app_with(registry, mock.clone());

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
        let app = app_with(registry, Arc::new(FailingPublisher));

        let (status, _body) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    fn app_with_curve(
        registry: Arc<DeviceRegistry>,
        tracker: Arc<niles_scheduler::ManualModeTracker>,
    ) -> Router {
        router(
            AppState::new(
                registry,
                Arc::new(MockPublisher::default()),
                Arc::new(CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_manual_mode(Some(tracker)),
        )
    }

    fn id_of(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn dimming_from_the_ui_holds_against_the_curve() {
        // The reported bug: a brightness set from the dashboard was
        // back on the curve within the minute, because the curve only
        // ever heard about voice, the dimmer and scenes.
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_dimmable("office", "desk_lamp"));
        let tracker = Arc::new(niles_scheduler::ManualModeTracker::new());
        let app = app_with_curve(registry, tracker.clone());

        let (status, _) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"brightness": 30}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(tracker.is_flagged(&id_of("office", "desk_lamp")));
    }

    #[tokio::test]
    async fn turning_a_light_on_leaves_it_to_the_curve() {
        // Asking for a light is not asking to own it. It is also how
        // you hand one back, so flagging here would make the curve
        // unreachable from the dashboard.
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_dimmable("office", "desk_lamp"));
        let tracker = Arc::new(niles_scheduler::ManualModeTracker::new());
        let app = app_with_curve(registry, tracker.clone());

        post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"on": true}),
        )
        .await;
        assert!(!tracker.is_flagged(&id_of("office", "desk_lamp")));
    }

    #[tokio::test]
    async fn a_room_command_holds_only_what_it_actually_set() {
        // A room narrows per device, so a colour meant for the strip
        // reaches the plain bulb as nothing at all. Flagging that bulb
        // would exempt it from the curve over a command it never got.
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_light("office", "strip"));
        registry.upsert(make_dimmable("office", "ceiling"));
        let tracker = Arc::new(niles_scheduler::ManualModeTracker::new());
        let app = app_with_curve(registry, tracker.clone());

        let (status, _) = post(
            app,
            "/rooms/office",
            serde_json::json!({"rgb": [255, 0, 0]}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(tracker.is_flagged(&id_of("office", "strip")));
        assert!(!tracker.is_flagged(&id_of("office", "ceiling")));
    }

    #[tokio::test]
    async fn without_a_curve_there_is_nothing_to_hold_against() {
        // `niles api` serves these routes with no scheduler behind
        // them. Commands still go out; there is just nothing to flag.
        let registry = Arc::new(DeviceRegistry::new());
        registry.upsert(make_dimmable("office", "desk_lamp"));
        let app = app_with(registry, Arc::new(MockPublisher::default()));

        let (status, _) = post(
            app,
            "/rooms/office/desk_lamp",
            serde_json::json!({"brightness": 30}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn tado_status_says_so_when_there_is_nowhere_to_keep_a_token() {
        // No database is a normal state for a laptop, and the card has
        // to be able to draw it rather than read it as a failure.
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let (status, body) = get_json(app, "/presence/tado").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["connectable"], false);
        assert_eq!(body["authorised"], false);
        assert_eq!(body["presence_enabled"], false);
    }

    #[tokio::test]
    async fn connecting_tado_without_a_database_is_not_a_500() {
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let (status, _) = post(app, "/presence/tado/connect", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn secrets_are_listed_even_with_nowhere_to_save_them() {
        // A laptop with no database still has to be able to draw the
        // page, and to say which credentials are coming from the
        // environment.
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let (status, body) = get_json(app, "/secrets").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["writable"], false);
        assert!(
            body["secrets"].as_array().unwrap().len() >= 9,
            "the list is what Niles reads, not what happens to be stored"
        );
        assert_eq!(
            body["secrets"][0]["source"], "unset",
            "with no config store to ask, nothing can be resolved"
        );
        assert!(
            body["secrets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["key"] == "auth.github_client_id"),
            "the client id is not secret, but it still has to be settable"
        );
    }

    #[tokio::test]
    async fn a_secret_cannot_be_read_back() {
        // There is no route for it, on purpose: a route that returns a
        // secret is a route that can be made to return it to somebody
        // else. The path exists for PUT and DELETE, so a GET is refused
        // by method rather than by path — which is the more useful of
        // the two answers.
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let (status, _) = get_json(app, "/secrets/mqtt.password").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn saving_without_a_store_says_why_rather_than_500ing() {
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/secrets/mqtt.password")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"value":"hunter2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn a_key_niles_does_not_read_is_refused() {
        // Otherwise a typo fills the table with secrets nothing will
        // ever look for, and the page quietly lies about being set up.
        let app = app_with(
            Arc::new(DeviceRegistry::new()),
            Arc::new(MockPublisher::default()),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/secrets/mqtt.passwrod")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"value":"hunter2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

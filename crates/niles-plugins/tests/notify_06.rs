#![allow(clippy::unwrap_used, reason = "test helpers: a failure fails the test")]

use async_trait::async_trait;
use niles_core::{Device, DeviceClass, DeviceId, DeviceRegistry, DeviceState, EventBus, RoomName};
use niles_notifications::{NotificationCenter, NotificationDelivery, Priority, TransportOutcome};
use niles_plugins::{House, Limits, MemoryStore, Runtime, Status};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MANIFEST: &str = include_str!("fixtures/notify-0.6/niles-integration.toml");
const COMPONENT: &[u8] = include_bytes!("fixtures/notify-0.6.wasm");

struct Delivery {
    outcome: TransportOutcome,
    gate: Option<Arc<tokio::sync::Semaphore>>,
    calls: Mutex<Vec<(String, Option<String>, Priority)>>,
    started: tokio::sync::Notify,
}

#[async_trait]
impl NotificationDelivery for Delivery {
    async fn deliver(
        &self,
        text: &str,
        room: Option<&str>,
        priority: Priority,
    ) -> TransportOutcome {
        self.calls
            .lock()
            .unwrap()
            .push((text.into(), room.map(str::to_owned), priority));
        self.started.notify_one();
        if let Some(gate) = &self.gate {
            let _permit = gate.acquire().await.unwrap();
        }
        self.outcome
    }
}

async fn install(manifest: &str, delivery: Arc<Delivery>) -> Runtime {
    let registry = Arc::new(DeviceRegistry::new());
    registry.upsert(Device::new(
        DeviceId::parse("test:office/lamp").unwrap(),
        DeviceState::default(),
        DeviceClass::Light,
    ));
    let house = House::new(registry, EventBus::new(32), Arc::new(MemoryStore::new()));
    let runtime = Runtime::new(house, Limits::default()).unwrap();
    runtime.connect_notification_rooms([RoomName::parse("kitchen").unwrap()]);
    let center = NotificationCenter::new(8).with_delivery(delivery);
    let center = if manifest.contains("quiet fixture") {
        use chrono::Timelike;
        let hour = chrono::Utc::now().hour();
        center.with_quiet_hours(niles_notifications::QuietHoursConfig {
            enabled: true,
            window: Some(niles_notifications::QuietWindow::new(
                chrono::NaiveTime::from_hms_opt((hour + 23) % 24, 0, 0).unwrap(),
                chrono::NaiveTime::from_hms_opt((hour + 2) % 24, 0, 0).unwrap(),
            )),
            timezone: None,
        })
    } else {
        center
    };
    runtime.connect_notifications(Arc::new(center));
    runtime.install(manifest, COMPONENT.to_vec()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if runtime
                .info("notifyfixture")
                .is_some_and(|info| info.status == Status::Running)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fixture starts");
    runtime
}

async fn call(runtime: &Runtime, tool: &str, args: Value) -> Result<String, String> {
    runtime
        .call_tool("notifyfixture", tool, &args.to_string())
        .await
}

async fn status(runtime: &Runtime, id: u64) -> String {
    serde_json::from_str::<Value>(
        &call(runtime, "status", json!({"request":id}))
            .await
            .unwrap(),
    )
    .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn wait_status(runtime: &Runtime, id: u64, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let got = status(runtime, id).await;
            if got == expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("request {id} did not reach {expected}"));
}

fn parse_id(value: &str) -> u64 {
    serde_json::from_str::<Value>(value).unwrap()["request"]
        .as_u64()
        .unwrap()
}

fn delivery(outcome: TransportOutcome, gate: bool) -> Arc<Delivery> {
    Arc::new(Delivery {
        outcome,
        gate: gate.then(|| Arc::new(tokio::sync::Semaphore::new(0))),
        calls: Mutex::new(Vec::new()),
        started: tokio::sync::Notify::new(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn component_exports_drive_tracked_delivery_lifecycle_and_outcomes() {
    for (outcome, expected) in [
        (TransportOutcome::Completed, "RequestStatus::Completed"),
        (TransportOutcome::Partial, "RequestStatus::Partial"),
        (TransportOutcome::Failed, "RequestStatus::Failed"),
    ] {
        let fake = delivery(outcome, true);
        let runtime = install(MANIFEST, fake.clone()).await;
        let result = call(
            &runtime,
            "request",
            json!({"message":"synthetic notice","room":"kitchen"}),
        )
        .await
        .unwrap();
        let id = parse_id(&result);
        assert_eq!(status(&runtime, id).await, "RequestStatus::Pending");
        tokio::time::timeout(Duration::from_secs(2), fake.started.notified())
            .await
            .expect("fake transport has started");
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            &[(
                "synthetic notice".into(),
                Some("kitchen".into()),
                Priority::Important
            )]
        );
        fake.gate.as_ref().unwrap().add_permits(1);
        wait_status(&runtime, id, expected).await;
        assert_eq!(
            status(&runtime, id).await,
            expected,
            "terminal status is repeatable"
        );
        runtime.shutdown().await;
    }
    let fake = delivery(TransportOutcome::Completed, false);
    let quiet_manifest = MANIFEST.replace("description = \"Exercises tracked notification requests through the 0.6 component exports.\"", "description = \"quiet fixture\"");
    let runtime = install(&quiet_manifest, fake.clone()).await;
    let id = parse_id(
        &call(
            &runtime,
            "request",
            json!({"message":"quiet","priority":"low"}),
        )
        .await
        .unwrap(),
    );
    wait_status(&runtime, id, "RequestStatus::Suppressed").await;
    assert!(fake.calls.lock().unwrap().is_empty());
    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn component_export_refuses_bad_room_and_missing_notify_permission() {
    let fake = delivery(TransportOutcome::Completed, false);
    let runtime = install(MANIFEST, fake.clone()).await;
    let unknown = call(
        &runtime,
        "request",
        json!({"message":"no route","room":"study"}),
    )
    .await
    .unwrap_err();
    assert!(unknown.contains("unknown room"), "{unknown}");
    assert!(unknown.contains("study"), "{unknown}");
    let unrouted = call(
        &runtime,
        "request",
        json!({"message":"no route","room":"office"}),
    )
    .await
    .unwrap_err();
    assert!(unrouted.contains("no configured satellite"), "{unrouted}");
    assert!(unrouted.contains("office"), "{unrouted}");
    assert!(fake.calls.lock().unwrap().is_empty());
    runtime.shutdown().await;

    let denied_manifest = MANIFEST.replace("notify = true", "");
    let denied = install(&denied_manifest, fake.clone()).await;
    let error = call(&denied, "request", json!({"message":"denied"}))
        .await
        .unwrap_err();
    assert!(error.contains("NotPermitted"), "{error}");
    assert!(fake.calls.lock().unwrap().is_empty());
    denied.shutdown().await;
}

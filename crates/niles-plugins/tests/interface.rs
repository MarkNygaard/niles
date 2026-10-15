//! Interface versions side by side: `fixtures/relay.wasm`, built against
//! 0.2.0 and kept so, still loads and runs on a Niles that serves 0.2.1;
//! `fixtures/printer.wasm`, built against 0.2.1, uses what 0.2.1 added.

use niles_core::{DeviceId, DeviceRegistry, Event, EventBus};
use niles_plugins::{House, Limits, MemoryStore, PluginStore, Runtime, Status};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PRINTER_MANIFEST: &str = include_str!("fixtures/printer/niles-integration.toml");
const PRINTER: &[u8] = include_bytes!("fixtures/printer.wasm");
const RELAY_MANIFEST: &str = include_str!("fixtures/relay/niles-integration.toml");
const RELAY: &[u8] = include_bytes!("fixtures/relay.wasm");

struct Home {
    runtime: Runtime,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    store: Arc<MemoryStore>,
}

impl Home {
    fn new(settings: toml::Table, store: Arc<MemoryStore>) -> Self {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::new(1024);
        let house = House::new(registry.clone(), bus.clone(), store.clone())
            .with_settings(Arc::new(move |_| settings.clone()))
            .with_time_zone(Arc::new(|| "Europe/Copenhagen".to_string()));
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
    let report =
        niles_plugins::check::check(PRINTER_MANIFEST, Some(PRINTER.to_vec()), None).await;
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

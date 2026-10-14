//! The runtime against a real component: `fixtures/hello.wasm`, built
//! from `fixtures/hello/` (see its `build.sh`). No network is used; the
//! HTTP tests are about what is refused before anything is sent.

use niles_core::{
    Device, DeviceClass, DeviceId, DeviceRegistry, DeviceState, Event, EventBus, PresenceState,
};
use niles_mqtt::CommandRouter;
use niles_plugins::{
    ActionValue, Context, House, Limits, MemoryStore, PluginStore, Runtime, Status, SurfaceError,
};
use niles_tools::{DynamicTools, ToolRegistry};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MANIFEST: &str = include_str!("fixtures/hello/niles-integration.toml");
const COMPONENT: &[u8] = include_bytes!("fixtures/hello.wasm");

/// The spec's limits, shortened so a test that waits for a deadline or
/// a backoff takes milliseconds.
fn quick() -> Limits {
    Limits {
        call: Duration::from_secs(5),
        init: Duration::from_secs(10),
        render: Duration::from_secs(3),
        stop: Duration::from_secs(1),
        backoff: vec![Duration::from_millis(50), Duration::from_millis(100)],
        restarts_per_hour: 2,
        init_retry: Duration::from_millis(200),
        tick: Duration::from_millis(10),
    }
}

struct House_ {
    runtime: Runtime,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    tools: DynamicTools,
    store: Arc<MemoryStore>,
    settings: Arc<Mutex<toml::Table>>,
}

impl House_ {
    async fn new(limits: Limits, settings: toml::Table) -> Self {
        Self::with_store(limits, settings, Arc::new(MemoryStore::new())).await
    }

    async fn with_store(limits: Limits, settings: toml::Table, store: Arc<MemoryStore>) -> Self {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::new(1024);
        let tools = DynamicTools::new();
        let settings = Arc::new(Mutex::new(settings));
        let read = settings.clone();
        let house = House::new(registry.clone(), bus.clone(), store.clone())
            .with_tools(tools.clone())
            .with_settings(Arc::new(move |_| read.lock().unwrap().clone()))
            .with_secrets(Arc::new(|id, key| {
                (id == "hello" && key == "token").then(|| "s3cret".to_string())
            }));
        let runtime = Runtime::new(house, limits).expect("the runtime starts");
        Self {
            runtime,
            registry,
            bus,
            tools,
            store,
            settings,
        }
    }

    async fn install(&self) {
        self.runtime
            .install(MANIFEST, COMPONENT.to_vec())
            .await
            .expect("hello installs");
    }

    async fn running() -> Self {
        let house = Self::new(quick(), toml::Table::new()).await;
        house.install().await;
        house.wait_for(|s| *s == Status::Running).await;
        house
    }

    async fn wait_for(&self, wanted: impl Fn(&Status) -> bool) -> Status {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let status = self.runtime.info("hello").expect("hello is loaded").status;
            if wanted(&status) {
                return status;
            }
            assert!(Instant::now() < until, "still {status:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn tool(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        self.runtime
            .call_tool("hello", name, &args.to_string())
            .await
    }

    fn device(&self, id: &str) -> Option<Device> {
        self.registry.get(&DeviceId::parse(id).unwrap())
    }
}

fn table(pairs: &[(&str, &str)]) -> toml::Table {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), toml::Value::String(v.to_string())))
        .collect()
}

async fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<Event>,
    wanted: impl Fn(&Event) -> bool,
) -> Event {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("the bus is open");
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("the event arrives")
}

#[tokio::test(flavor = "multi_thread")]
async fn init_reads_its_settings_and_secrets() {
    let house = House_::new(quick(), table(&[("greeting", "Hej"), ("room", "kitchen")])).await;
    house.install().await;
    house.wait_for(|s| *s == Status::Running).await;
    assert_eq!(
        house.tool("greet", json!({"name": "Mark"})).await.unwrap(),
        "Hej, Mark!"
    );
    assert!(house.device("x_hello:kitchen/lamp").is_some());
    assert_eq!(house.tool("secret", json!({})).await.unwrap(), "true");
}

#[tokio::test(flavor = "multi_thread")]
async fn init_saying_what_is_wrong_is_the_status_line() {
    let house = House_::new(quick(), table(&[("greeting", "fail")])).await;
    house.install().await;
    let status = house.wait_for(|s| matches!(s, Status::Failing(_))).await;
    assert_eq!(status, Status::Failing("The greeting is fail.".into()));
    assert_eq!(status.line(), "Not working: The greeting is fail.");

    // Fixed settings and a restart, as saving the setup panel does.
    *house.settings.lock().unwrap() = table(&[("greeting", "Hi")]);
    house.runtime.restart("hello").unwrap();
    house.wait_for(|s| *s == Status::Running).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_required_setting_is_not_set_up() {
    let manifest = MANIFEST.replace(
        "key = \"address\"\nlabel = \"Address\"\ntype = \"string\"\n",
        "key = \"address\"\nlabel = \"Address\"\ntype = \"string\"\nrequired = true\n",
    );
    assert_ne!(
        manifest, MANIFEST,
        "the fixture still has the address setting"
    );
    let house = House_::new(quick(), toml::Table::new()).await;
    house
        .runtime
        .install(&manifest, COMPONENT.to_vec())
        .await
        .unwrap();
    let status = house.wait_for(|s| matches!(s, Status::NotSetUp(_))).await;
    assert_eq!(status.line(), "Not set up: Address is required");

    *house.settings.lock().unwrap() = table(&[("address", "192.168.1.50")]);
    house.runtime.restart("hello").unwrap();
    house.wait_for(|s| *s == Status::Running).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_are_offered_by_keyword_and_called_through_the_shared_registry() {
    let house = House_::running().await;
    let mut registry = ToolRegistry::new();
    registry.attach_dynamic(house.tools.clone());

    let sent: Vec<String> = registry
        .llm_tools_for("say hello to Mark")
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(sent.contains(&"hello_greet".to_string()), "{sent:?}");
    let unrelated: Vec<String> = registry
        .llm_tools_for("what's the weather like")
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(!unrelated.iter().any(|n| n.starts_with("hello_")));

    let call = niles_llm::ToolCall {
        id: "1".into(),
        name: "hello_greet".into(),
        arguments: json!({"name": "Mark"}),
    };
    assert_eq!(
        registry.execute(&call).await.unwrap(),
        json!("Hello, Mark!")
    );

    // A tool's error is the plugin's sentence, for the model to say.
    let call = niles_llm::ToolCall {
        id: "2".into(),
        name: "hello_recall".into(),
        arguments: json!({}),
    };
    let err = registry.execute(&call).await.unwrap_err();
    assert_eq!(err.to_string(), "Which key?");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_with_readings_joins_the_house_and_its_changes_are_published() {
    let house = House_::running().await;
    let meter = house
        .device("x_hello:system/meter")
        .expect("the meter is registered");
    assert_eq!(meter.class, DeviceClass::Sensor);
    assert_eq!(meter.state.readings["power"].value, 340.0);
    assert_eq!(meter.state.readings["power"].unit.as_deref(), Some("W"));
    let lamp = house.device("x_hello:office/lamp").unwrap();
    assert!(lamp.supports_color_temperature());

    let mut rx = house.bus.subscribe();
    assert_eq!(
        house.tool("reading", json!({"watts": 5})).await.unwrap(),
        "reported"
    );
    let event = next_event(&mut rx, |e| matches!(e, Event::DeviceStateChanged { .. })).await;
    let Event::DeviceStateChanged { id, state } = event else {
        unreachable!()
    };
    assert_eq!(id.to_string(), "x_hello:system/meter");
    assert_eq!(state.readings["power"].value, 5.0, "only what changed");
    assert_eq!(
        house.device("x_hello:system/meter").unwrap().state.readings["power"].value,
        5.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_button_press_is_an_action_on_the_bus() {
    let house = House_::running().await;
    let mut rx = house.bus.subscribe();
    assert_eq!(house.tool("press", json!({})).await.unwrap(), "pressed");
    let event = next_event(&mut rx, |e| matches!(e, Event::DeviceAction { .. })).await;
    let Event::DeviceAction { id, action } = event else {
        unreachable!()
    };
    assert_eq!(id.to_string(), "x_hello:office/button");
    assert_eq!(action, "single");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_to_a_plugin_device_reaches_its_set_device() {
    let house = House_::running().await;
    let router =
        CommandRouter::z2m_only("zigbee2mqtt").with_commander(Arc::new(house.runtime.clone()));
    let lamp = DeviceId::parse("x_hello:office/lamp").unwrap();
    let desired = DeviceState {
        on: Some(true),
        brightness: Some(60),
        ..Default::default()
    };
    assert!(router.format(&lamp, &desired).is_none(), "nothing for MQTT");
    let commander = router
        .commander_for(&lamp, &desired)
        .expect("the plugin owns it");
    commander.command(&lamp, &desired).await.unwrap();
    let state = house.device("x_hello:office/lamp").unwrap().state;
    assert_eq!((state.on, state.brightness), (Some(true), Some(60)));

    let meter = DeviceId::parse("x_hello:system/meter").unwrap();
    let err = commander.command(&meter, &desired).await.unwrap_err();
    assert!(err.contains("cannot be switched"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_fires() {
    let house = House_::running().await;
    assert_eq!(
        house.tool("timer", json!({"seconds": 1})).await.unwrap(),
        "set"
    );
    let until = Instant::now() + Duration::from_secs(5);
    while house.tool("ticks", json!({})).await.unwrap() == "0" {
        assert!(Instant::now() < until, "the timer never fired");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let meter = house.device("x_hello:system/meter").unwrap();
    assert_eq!(meter.state.readings["ticks"].value, 1.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn timers_have_a_floor() {
    let house = House_::running().await;
    let err = house
        .tool("timer", json!({"seconds": 1, "every": true}))
        .await
        .unwrap_err();
    assert!(err.contains("at least 5 seconds"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_round_trips_and_the_quota_holds() {
    let house = House_::running().await;
    house
        .tool(
            "remember",
            json!({"key": "day:2026-10-14", "value": "cheap"}),
        )
        .await
        .unwrap();
    house
        .tool(
            "remember",
            json!({"key": "day:2026-10-13", "value": "dear"}),
        )
        .await
        .unwrap();
    assert_eq!(
        house
            .tool("recall", json!({"key": "day:2026-10-14"}))
            .await
            .unwrap(),
        "cheap"
    );
    assert_eq!(
        house.tool("keys", json!({"prefix": "day:"})).await.unwrap(),
        r#"["day:2026-10-13","day:2026-10-14"]"#
    );
    assert_eq!(
        house
            .store
            .get("hello", "day:2026-10-14")
            .await
            .unwrap()
            .as_deref(),
        Some("cheap"),
        "kept by Niles, under the plugin's id"
    );

    let near_half = 600 * 1024;
    house
        .tool("remember", json!({"key": "a", "size": near_half}))
        .await
        .unwrap();
    let err = house
        .tool("remember", json!({"key": "b", "size": near_half}))
        .await
        .unwrap_err();
    assert!(err.contains("Limit"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn http_outside_the_manifest_is_refused() {
    let house = House_::running().await;
    for url in [
        "https://not-declared.example.org/",
        "http://example.com/",
        "https://127.0.0.1/",
        "http://169.254.169.254/latest/meta-data",
        "http://192.168.1.50/",
    ] {
        let err = house
            .tool("fetch", json!({ "url": url }))
            .await
            .unwrap_err();
        assert!(err.contains("NotPermitted"), "{url}: {err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_past_its_deadline_restarts_the_plugin_and_repeated_failures_stop_it() {
    let house = House_::new(
        Limits {
            call: Duration::from_millis(300),
            ..quick()
        },
        toml::Table::new(),
    )
    .await;
    house.install().await;
    house.wait_for(|s| *s == Status::Running).await;

    let err = house.tool("spin", json!({})).await.unwrap_err();
    assert_eq!(err, "Hello is not working right now.");
    let info = house.runtime.info("hello").unwrap();
    assert_eq!(
        info.error.as_deref(),
        Some("timed out in call-tool after 300 ms")
    );
    assert!(
        !house.device("x_hello:office/lamp").unwrap().available,
        "its devices are unavailable while it is down"
    );

    // Started again after the backoff, devices back.
    house.wait_for(|s| *s == Status::Running).await;
    assert!(house.device("x_hello:office/lamp").unwrap().available);

    // Two restarts allowed within the hour here; the third failure stops it.
    house.tool("spin", json!({})).await.unwrap_err();
    house.wait_for(|s| *s == Status::Running).await;
    house.tool("spin", json!({})).await.unwrap_err();
    let status = house.wait_for(|s| matches!(s, Status::Stopped(_))).await;
    assert_eq!(status.line(), "Stopped after repeated failures");
    assert_eq!(
        house.tool("greet", json!({})).await.unwrap_err(),
        "Hello is not working right now."
    );

    // Somebody presses Restart.
    house.runtime.restart("hello").unwrap();
    house.wait_for(|s| *s == Status::Running).await;
    assert_eq!(
        house.tool("greet", json!({})).await.unwrap(),
        "Hello, world!"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panic_is_a_trap() {
    let house = House_::running().await;
    house.tool("panic", json!({})).await.unwrap_err();
    let info = house.runtime.info("hello").unwrap();
    let error = info.error.unwrap();
    assert!(error.starts_with("trapped in call-tool:"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_past_the_cap_is_a_trap() {
    let house = House_::running().await;
    house
        .tool("remember", json!({"key": "big", "size": 100 * 1024 * 1024}))
        .await
        .unwrap_err();
    let error = house.runtime.info("hello").unwrap().error.unwrap();
    assert!(error.contains("out of memory"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_renders_is_cached_and_answers_its_controls() {
    let house = House_::running().await;
    let mark = Context {
        room: None,
        person: Some("mark".into()),
    };
    let page = house
        .runtime
        .render("hello", "page", mark.clone())
        .await
        .unwrap();
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json["nodes"][0]["type"], "stack");
    assert_eq!(json["nodes"][1]["label"], "Greeting");
    assert!(
        page.text().contains("Pressed 0 times, for mark."),
        "{}",
        page.text()
    );

    let outcome = house
        .runtime
        .act("hello", "page", "hello", ActionValue::Pressed, mark.clone())
        .await
        .unwrap();
    assert_eq!(outcome.toast.as_deref(), Some("Hello pressed"));
    assert!(outcome.rerender);
    let again = house
        .runtime
        .render("hello", "page", mark.clone())
        .await
        .unwrap();
    assert!(
        again.text().contains("Pressed 1 times"),
        "a rerender is not served from the cache"
    );

    let err = house
        .runtime
        .act("hello", "page", "nothing", ActionValue::Pressed, mark)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        SurfaceError::Plugin("That button does nothing.".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_render_is_kept_until_invalidated() {
    let house = House_::running().await;
    let office = Context {
        room: Some("office".into()),
        person: None,
    };
    let first = house
        .runtime
        .render("hello", "card", office.clone())
        .await
        .unwrap();
    house.tool("timer", json!({"seconds": 1})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let cached = house
        .runtime
        .render("hello", "card", office.clone())
        .await
        .unwrap();
    assert_eq!(first, cached, "ten phones open on one page cost one render");

    let mut rx = house.bus.subscribe();
    house.tool("invalidate", json!({})).await.unwrap();
    let event = next_event(&mut rx, |e| matches!(e, Event::UiInvalidated { .. })).await;
    let Event::UiInvalidated { plugin, surface } = event else {
        unreachable!()
    };
    assert_eq!((plugin.as_str(), surface.as_str()), ("hello", "page"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tree_that_breaks_the_rules_is_not_drawn() {
    let house = House_::running().await;
    let err = house
        .runtime
        .render("hello", "broken", Context::default())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, SurfaceError::Plugin(why) if why.contains("cannot be drawn")),
        "{err:?}"
    );
    let err = house
        .runtime
        .render("hello", "undeclared", Context::default())
        .await
        .unwrap_err();
    assert!(matches!(err, SurfaceError::NotFound(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribed_events_reach_the_plugin_and_others_do_not() {
    let house = House_::running().await;
    let door = DeviceId::parse("z2m:hall/door").unwrap();
    house.registry.upsert(Device::new(
        door.clone(),
        DeviceState::default(),
        DeviceClass::Contact,
    ));
    house.bus.publish(Event::DeviceStateChanged {
        id: door,
        state: DeviceState {
            open: Some(true),
            ..Default::default()
        },
    });
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let last = house.tool("last_event", json!({})).await.unwrap();
        if last == "state z2m:hall/door open=Some(true)" {
            break;
        }
        assert!(Instant::now() < until, "got {last:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Not a door: outside the plugin's */door pattern.
    house.bus.publish(Event::DeviceStateChanged {
        id: DeviceId::parse("z2m:hall/lamp").unwrap(),
        state: DeviceState {
            on: Some(true),
            ..Default::default()
        },
    });
    house.bus.publish(Event::PresenceChanged {
        state: PresenceState::Away,
    });
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let last = house.tool("last_event", json!({})).await.unwrap();
        if last == "presence HomeState::Away" {
            break;
        }
        assert!(!last.contains("lamp"), "a lamp is not a door");
        assert!(Instant::now() < until, "got {last:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_are_rate_limited_and_urgent_needs_its_own_permission() {
    let house = House_::running().await;
    for _ in 0..6 {
        assert_eq!(
            house.tool("say", json!({"message": "Hi."})).await.unwrap(),
            "said"
        );
    }
    let err = house
        .tool("say", json!({"message": "Hi."}))
        .await
        .unwrap_err();
    assert!(err.contains("Limit"), "{err}");
    let err = house
        .tool("say", json!({"message": "Now!", "urgent": true}))
        .await
        .unwrap_err();
    assert!(err.contains("NotPermitted"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_are_remembered_while_the_plugin_is_down() {
    let store = Arc::new(MemoryStore::new());
    let first = House_::with_store(quick(), toml::Table::new(), store.clone()).await;
    first.install().await;
    first.wait_for(|s| *s == Status::Running).await;
    first.runtime.shutdown().await;

    // Niles starts again; the plugin's init fails, so nothing re-reports.
    let second = House_::with_store(quick(), table(&[("greeting", "fail")]), store).await;
    assert_eq!(second.runtime.load_installed().await, 1);
    second.wait_for(|s| matches!(s, Status::Failing(_))).await;
    let lamp = second
        .device("x_hello:office/lamp")
        .expect("still in its room");
    assert!(!lamp.available);
    assert_eq!(lamp.class, DeviceClass::Light);
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_registered_lists_what_niles_holds() {
    let house = House_::running().await;
    assert_eq!(
        house.tool("registered", json!({})).await.unwrap(),
        r#"["office/button","office/lamp","system/meter"]"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_says_who_is_running() {
    let house = House_::running().await;
    let plugins = house.runtime.plugins();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "Hello");
    assert_eq!(plugins[0].version, "0.1.0");
    assert_eq!(plugins[0].status.kind(), "running");
}

#[tokio::test(flavor = "multi_thread")]
async fn check_reports_a_good_plugin_as_good() {
    let report = niles_plugins::check::check(MANIFEST, Some(COMPONENT.to_vec()), None).await;
    assert!(report.ok(), "{:?}", report.problems);
    assert!(report.tools.iter().any(|t| t.name == "hello_greet"));
    assert!(
        report
            .imports
            .iter()
            .any(|i| i.starts_with("niles:integration/devices@0.1"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_refuses_what_is_not_a_component() {
    let report =
        niles_plugins::check::check(MANIFEST, Some(b"\0asm not really".to_vec()), None).await;
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("not a WebAssembly component")),
        "{:?}",
        report.problems
    );
}

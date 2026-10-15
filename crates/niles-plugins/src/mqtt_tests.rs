//! MQTT against a real broker: rumqttd, embedded, on this machine, with
//! the 0.2 test plugin `tests/fixtures/relay.wasm` (built from
//! `tests/fixtures/relay/`) and the 0.1 one, `hello.wasm`, beside it.
//!
//! Here rather than in `tests/` because the broker is on loopback, which
//! the policy refuses, and the override for that exists only in the
//! test's [`House::allowing_loopback`].

use crate::manifest::Manifest;
use crate::policy::MqttPolicy;
use crate::{House, Limits, MemoryStore, PluginStore, Runtime};
use niles_core::{DeviceId, DeviceRegistry, EventBus};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};

const RELAY: &[u8] = include_bytes!("../tests/fixtures/relay.wasm");
const RELAY_MANIFEST: &str = include_str!("../tests/fixtures/relay/niles-integration.toml");
const HELLO: &[u8] = include_bytes!("../tests/fixtures/hello.wasm");
const HELLO_MANIFEST: &str = include_str!("../tests/fixtures/hello/niles-integration.toml");

const USER: &str = "bblp";
const PASSWORD: &str = "12345678";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

/// rumqttd on `plain` without TLS and, if given, on `tls` with that
/// certificate and key, both asking for [`USER`] and [`PASSWORD`]. It
/// runs on threads of its own until the test process ends.
async fn broker(plain: u16, tls: Option<(u16, &Path, &Path)>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let connections = rumqttd::ConnectionSettings {
        connection_timeout_ms: 60_000,
        max_payload_size: 1024 * 1024,
        max_inflight_count: 100,
        auth: Some(HashMap::from([(USER.to_string(), PASSWORD.to_string())])),
        external_auth: None,
        dynamic_filters: true,
    };
    let server = |name: &str, port: u16, tls: Option<rumqttd::TlsConfig>| rumqttd::ServerSettings {
        name: name.into(),
        listen: SocketAddr::from(([127, 0, 0, 1], port)),
        tls,
        next_connection_delay_ms: 1,
        connections: connections.clone(),
    };
    let mut servers = HashMap::from([("plain".to_string(), server("plain", plain, None))]);
    if let Some((port, cert, key)) = tls {
        let tls = rumqttd::TlsConfig::Rustls {
            capath: None,
            certpath: cert.display().to_string(),
            keypath: key.display().to_string(),
        };
        servers.insert("tls".into(), server("tls", port, Some(tls)));
    }
    let config = rumqttd::Config {
        id: 0,
        router: rumqttd::RouterConfig {
            max_connections: 100,
            max_outgoing_packet_count: 200,
            max_segment_size: 10 * 1024 * 1024,
            max_segment_count: 10,
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: Default::default(),
        },
        v4: Some(servers),
        v5: None,
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    };
    std::thread::spawn(move || {
        let _ = rumqttd::Broker::new(config).start();
    });
    for port in std::iter::once(plain).chain(tls.map(|(port, _, _)| port)) {
        let started = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the broker did not start"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// A self-signed certificate for 127.0.0.1, written as PEM files, and
/// the fingerprint Niles should pin for it.
fn certificate(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let made =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).expect("a certificate");
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    std::fs::write(&cert, made.cert.pem()).unwrap();
    std::fs::write(&key, made.signing_key.serialize_pem()).unwrap();
    let fingerprint = crate::mqtt::fingerprint(made.cert.der()).unwrap();
    (cert, key, fingerprint)
}

/// A TCP relay in front of the broker, whose connections can be cut,
/// as a network drop would.
struct Cable {
    port: u16,
    live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Cable {
    async fn to(upstream: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let tasks = live.clone();
        tokio::spawn(async move {
            while let Ok((mut down, _)) = listener.accept().await {
                let task = tokio::spawn(async move {
                    if let Ok(mut up) = TcpStream::connect(("127.0.0.1", upstream)).await {
                        let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
                    }
                });
                tasks.lock().unwrap().push(task);
            }
        });
        Self { port, live }
    }

    fn cut(&self) {
        for task in self.live.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

/// A client of the broker's own, as a device or another program would be.
struct Device {
    client: rumqttc::AsyncClient,
    received: tokio::sync::mpsc::UnboundedReceiver<(String, Vec<u8>)>,
}

impl Device {
    async fn on(port: u16, filter: &str) -> Self {
        let mut options = rumqttc::MqttOptions::new("device", "127.0.0.1", port);
        options.set_credentials(USER, PASSWORD);
        let (client, mut events) = rumqttc::AsyncClient::new(options, 64);
        let (tx, received) = tokio::sync::mpsc::unbounded_channel();
        let (ready, subscribed) = tokio::sync::oneshot::channel();
        let mut ready = Some(ready);
        tokio::spawn(async move {
            loop {
                match events.poll().await {
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                        let _ = tx.send((p.topic.clone(), p.payload.to_vec()));
                    }
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::SubAck(_))) => {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
            }
        });
        client
            .subscribe(filter, rumqttc::QoS::AtLeastOnce)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), subscribed)
            .await
            .expect("the device subscribed")
            .unwrap();
        Self { client, received }
    }

    async fn publish(&self, topic: &str, payload: &str) {
        self.client
            .publish(topic, rumqttc::QoS::AtLeastOnce, false, payload.as_bytes())
            .await
            .unwrap();
    }

    async fn next(&mut self) -> (String, String) {
        let (topic, payload) = tokio::time::timeout(Duration::from_secs(10), self.received.recv())
            .await
            .expect("a message in time")
            .unwrap();
        (topic, String::from_utf8(payload).unwrap())
    }
}

struct Home {
    runtime: Runtime,
    registry: Arc<DeviceRegistry>,
}

impl Home {
    /// A Niles whose relay plugin talks to `address` with `broker` (the
    /// manifest's `plain` or `device`), the manifest's ports being these.
    async fn new(store: Arc<MemoryStore>, address: &str, broker: &str) -> Self {
        let registry = Arc::new(DeviceRegistry::new());
        let mut relay = toml::Table::new();
        relay.insert("address".into(), address.into());
        relay.insert("broker".into(), broker.into());
        relay.insert("username".into(), USER.into());
        let house = House::new(registry.clone(), EventBus::new(1024), store.clone())
            .with_settings(Arc::new(move |id| match id {
                "relay" => relay.clone(),
                _ => toml::Table::new(),
            }))
            .with_secrets(Arc::new(|id, key| {
                (id == "relay" && key == "password").then(|| PASSWORD.to_string())
            }))
            .allowing_loopback();
        let runtime = Runtime::new(house, Limits::default()).expect("the runtime starts");
        Self { runtime, registry }
    }

    async fn install_relay(&self, plain: u16, device: u16) {
        self.runtime
            .install(&relay_manifest(plain, device), RELAY.to_vec())
            .await
            .expect("the relay installs");
        running(&self.runtime, "relay").await;
    }

    async fn tool(&self, name: &str, args: &str) -> String {
        self.runtime
            .call_tool("relay", name, args)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    async fn statuses(&self) -> Vec<String> {
        serde_json::from_str(&self.tool("statuses", "{}").await).unwrap()
    }

    /// Wait until the plugin has been told `n` statuses, and return them.
    async fn told(&self, n: usize) -> Vec<String> {
        let started = Instant::now();
        loop {
            let statuses = self.statuses().await;
            if statuses.len() >= n {
                return statuses;
            }
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "waited for {n} statuses, got {statuses:?}; log: {:?}",
                self.runtime.log("relay")
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn echo(&self) -> Option<f64> {
        let id = DeviceId::parse("x_relay:office/relay").unwrap();
        let started = Instant::now();
        loop {
            let value = self
                .registry
                .get(&id)
                .and_then(|d| d.state.readings.get("echo").map(|r| r.value));
            if value.is_some() || started.elapsed() > Duration::from_secs(10) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// Wait for the plugin's `init` to have returned.
async fn running(runtime: &Runtime, id: &str) {
    let started = Instant::now();
    loop {
        let status = runtime.info(id).map(|i| i.status);
        if status == Some(crate::Status::Running) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{id} is {status:?}; log: {:?}",
            runtime.log(id)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn relay_manifest(plain: u16, device: u16) -> String {
    RELAY_MANIFEST
        .replace("port = 18830", &format!("port = {plain}"))
        .replace("port = 18831", &format!("port = {device}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_becomes_a_reading_and_is_published_back_beside_a_0_1_plugin() {
    let port = free_port();
    broker(port, None).await;
    let mut device = Device::on(port, "relay/out/#").await;
    let home = Home::new(Arc::new(MemoryStore::new()), "127.0.0.1", "plain").await;
    home.runtime
        .install(HELLO_MANIFEST, HELLO.to_vec())
        .await
        .expect("the 0.1 plugin installs");
    running(&home.runtime, "hello").await;
    home.install_relay(port, free_port()).await;

    assert_eq!(home.told(1).await, ["connected"]);
    device.publish("relay/in/kitchen", "21.5").await;
    assert_eq!(home.echo().await, Some(21.5));
    assert_eq!(
        device.next().await,
        ("relay/out/kitchen".to_string(), "21.5".to_string())
    );

    let greeting = home
        .runtime
        .call_tool("hello", "greet", r#"{"name":"Mark"}"#)
        .await;
    assert_eq!(
        greeting.as_deref(),
        Ok("Hello, Mark!"),
        "0.1 runs beside 0.2"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_connection_comes_back_and_subscribes_again() {
    let port = free_port();
    broker(port, None).await;
    let cable = Cable::to(port).await;
    let mut device = Device::on(port, "relay/out/#").await;
    let home = Home::new(Arc::new(MemoryStore::new()), "127.0.0.1", "plain").await;
    home.install_relay(cable.port, free_port()).await;
    assert_eq!(home.told(1).await, ["connected"]);

    cable.cut();
    let statuses = home.told(3).await;
    assert!(statuses[1].starts_with("disconnected: "), "{statuses:?}");
    assert_eq!(statuses[2], "connected");

    device.publish("relay/in/hall", "7").await;
    assert_eq!(
        device.next().await,
        ("relay/out/hall".to_string(), "7".to_string()),
        "the subscription was made again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_password_stops_the_trying() {
    // rumqttd hangs up on a wrong password; Mosquitto and the printers
    // answer CONNACK with a refusal, as this stand-in does.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let mut connect = [0u8; 256];
            let _ = socket.read(&mut connect).await;
            let _ = socket.write_all(&[0x20, 0x02, 0x00, 0x04]).await;
        }
    });
    let home = Home::new(Arc::new(MemoryStore::new()), "127.0.0.1", "plain").await;
    home.install_relay(port, free_port()).await;
    assert_eq!(
        home.told(1).await,
        ["failed: the broker refused the username or password"]
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "no second try");
}

#[tokio::test(flavor = "multi_thread")]
async fn trust_device_pins_the_first_key_and_asks_before_another() {
    let dir = tempfile::tempdir().unwrap();
    let (cert, key, fingerprint) = certificate(dir.path());
    let (plain, tls) = (free_port(), free_port());
    broker(plain, Some((tls, &cert, &key))).await;
    let store = Arc::new(MemoryStore::new());

    let home = Home::new(store.clone(), "127.0.0.1", "device").await;
    home.install_relay(plain, tls).await;
    assert_eq!(home.told(1).await, ["connected"]);
    let pinned = store.keys("relay", ".niles/").await.unwrap();
    assert_eq!(pinned.len(), 1, "{pinned:?}");
    assert_eq!(
        store.get("relay", &pinned[0]).await.unwrap(),
        Some(fingerprint.clone())
    );
    home.runtime.shutdown().await;

    // As if the device had shown another key before: a firmware update,
    // or another device at its address.
    let before = "00:11:22:33".to_string();
    store.set("relay", &pinned[0], &before).await.unwrap();
    let home = Home::new(store.clone(), "127.0.0.1", "device").await;
    home.install_relay(plain, tls).await;
    assert_eq!(
        home.told(1).await,
        [format!("certificate-changed {before} {fingerprint}")]
    );
    assert_eq!(home.tool("accept", "{}").await, "accepted");
    assert_eq!(home.told(2).await[1], "connected");
    assert_eq!(
        store.get("relay", &pinned[0]).await.unwrap(),
        Some(fingerprint)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_cannot_see_or_touch_the_pins() {
    let manifest = Manifest::parse(&relay_manifest(1883, 8883)).unwrap();
    let policy = MqttPolicy::new(&manifest.permissions, |_| Some("192.168.1.40".into()));
    let pin = crate::mqtt::pin_key(&policy.target("device").unwrap().unwrap());
    let store = Arc::new(MemoryStore::new());
    store.set("hello", &pin, "00:11").await.unwrap();
    store.set("hello", "mine", "x").await.unwrap();
    let home = Home::new(store.clone(), "127.0.0.1", "plain").await;
    home.runtime
        .install(HELLO_MANIFEST, HELLO.to_vec())
        .await
        .unwrap();
    running(&home.runtime, "hello").await;
    let call = |tool: &'static str, args: String| {
        let runtime = home.runtime.clone();
        async move { runtime.call_tool("hello", tool, &args).await }
    };
    assert_eq!(
        call("keys", "{}".into()).await.as_deref(),
        Ok(r#"["mine"]"#)
    );
    let key = serde_json::json!({ "key": pin, "value": "forged" }).to_string();
    assert_eq!(call("recall", key.clone()).await.as_deref(), Ok("nothing"));
    assert!(
        call("remember", key)
            .await
            .unwrap_err()
            .contains("Niles's own")
    );
    assert_eq!(
        store.get("hello", &pin).await.unwrap().as_deref(),
        Some("00:11")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_must_match_its_manifests_interface() {
    let home = Home::new(Arc::new(MemoryStore::new()), "127.0.0.1", "plain").await;
    let mut lines = relay_manifest(1883, 8883)
        .lines()
        .filter(|l| !l.starts_with("mqtt = [") && !l.contains("{ name = ") && l.trim() != "]")
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    lines = lines.replace("interface = \"0.2\"", "interface = \"0.1\"");
    let refused = home
        .runtime
        .install(&lines, RELAY.to_vec())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("built for niles:integration 0.2"),
        "{refused}"
    );

    let hello = HELLO_MANIFEST.replace("interface = \"0.1\"", "interface = \"0.2\"");
    let refused = home
        .runtime
        .install(&hello, HELLO.to_vec())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("built for niles:integration 0.1"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_says_which_world_a_component_targets() {
    let report = crate::check::check(&relay_manifest(1883, 8883), Some(RELAY.to_vec()), None).await;
    assert!(report.ok(), "{:?}", report.problems);
    assert!(
        report
            .passed
            .iter()
            .any(|p| p.contains("niles:integration/plugin@0.2")),
        "{:?}",
        report.passed
    );
    assert!(
        report
            .imports
            .iter()
            .any(|i| i == "niles:integration/mqtt@0.2.0")
    );
}

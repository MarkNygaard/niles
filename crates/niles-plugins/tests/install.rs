//! Installing from GitHub, against a stand-in GitHub: previews, the
//! two-step install, update checks and updates, rollback, disable and
//! remove. The component is `fixtures/hello.wasm`, released under
//! whatever versions a test needs by changing its manifest.

use async_trait::async_trait;
use niles_core::{Device, DeviceClass, DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_plugins::{
    GitHub, House, Installer, Leftovers, Limits, MemoryStore, Options, PluginStore, Refusal,
    Runtime, Status,
};
use niles_tools::DynamicTools;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = include_str!("fixtures/hello/niles-integration.toml");
const COMPONENT: &[u8] = include_bytes!("fixtures/hello.wasm");
const LOGO: &str = r#"<?xml version="1.0"?>
<!-- the hello mark -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="currentColor"/></svg>"#;

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

fn options() -> Options {
    Options {
        rollback_window: Duration::from_secs(30),
        watch_every: Duration::from_millis(20),
        ..Options::default()
    }
}

/// The fixture's manifest at another version, optionally with a logo
/// and with something else changed.
fn manifest(version: &str) -> String {
    MANIFEST.replace("version = \"0.1.0\"", &format!("version = \"{version}\""))
}

fn with_logo(manifest: &str) -> String {
    manifest.replace(
        "interface = \"0.1\"",
        "interface = \"0.1\"\nlogo = \"assets/logo.svg\"",
    )
}

/// One asset on a release: its name, its bytes, and what GitHub says
/// about it.
struct Asset {
    name: &'static str,
    body: Vec<u8>,
    /// The size GitHub reports, when it is not the body's.
    size: Option<u64>,
    digest: Option<String>,
}

fn asset(name: &'static str, body: impl Into<Vec<u8>>) -> Asset {
    let body = body.into();
    Asset {
        name,
        digest: Some(format!("sha256:{}", niles_db::hex_digest(&body))),
        body,
        size: None,
    }
}

fn plugin_assets(manifest: &str) -> Vec<Asset> {
    let mut assets = vec![
        asset("niles-integration.toml", manifest),
        asset("integration.wasm", COMPONENT),
    ];
    if manifest.contains("logo =") {
        assets.push(asset("logo.svg", LOGO));
    }
    assets
}

/// A stand-in GitHub with one repo, `jane/hello`.
struct Hub {
    server: MockServer,
}

impl Hub {
    async fn new() -> Self {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/jane/hello"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "jane/hello"})))
            .mount(&server)
            .await;
        Self { server }
    }

    /// Put the assets up for download and describe the release.
    async fn release(&self, tag: &str, prerelease: bool, assets: Vec<Asset>) -> serde_json::Value {
        let mut described = Vec::new();
        for asset in assets {
            let at = format!("/download/{tag}/{}", asset.name);
            Mock::given(method("GET"))
                .and(path(at.clone()))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(asset.body.clone()))
                .mount(&self.server)
                .await;
            described.push(json!({
                "name": asset.name,
                "size": asset.size.unwrap_or(asset.body.len() as u64),
                "browser_download_url": format!("{}{at}", self.server.uri()),
                "digest": asset.digest,
            }));
        }
        json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/jane/hello/releases/tag/{tag}"),
            "body": format!("What is new in {tag}."),
            "draft": false,
            "prerelease": prerelease,
            "assets": described,
        })
    }

    async fn answer(&self, at: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    /// `tag` as the latest release, and as itself.
    async fn latest(&self, tag: &str, assets: Vec<Asset>) {
        let release = self.release(tag, false, assets).await;
        self.answer(&format!("/repos/jane/hello/releases/tags/{tag}"), 200, release.clone())
            .await;
        self.answer("/repos/jane/hello/releases/latest", 200, release)
            .await;
    }

    fn github(&self) -> GitHub {
        GitHub::at(&self.server.uri()).unwrap()
    }
}

#[derive(Default)]
struct Forgotten(Mutex<Vec<String>>);

#[async_trait]
impl Leftovers for Forgotten {
    async fn forget(&self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(id.to_string());
        Ok(())
    }
}

struct Home {
    installer: Installer,
    runtime: Runtime,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    tools: DynamicTools,
    store: Arc<MemoryStore>,
    forgotten: Arc<Forgotten>,
}

impl Home {
    async fn new(hub: &Hub, options: Options) -> Self {
        Self::build(hub, options, |i| i).await
    }

    async fn build(hub: &Hub, options: Options, edit: impl FnOnce(Installer) -> Installer) -> Self {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::new(1024);
        let tools = DynamicTools::new();
        let store = Arc::new(MemoryStore::new());
        let house = House::new(registry.clone(), bus.clone(), store.clone()).with_tools(tools.clone());
        let runtime = Runtime::new(house, quick()).unwrap();
        let forgotten = Arc::new(Forgotten::default());
        let installer = edit(
            Installer::new(runtime.clone(), hub.github(), options)
                .with_leftovers(forgotten.clone()),
        );
        Self {
            installer,
            runtime,
            registry,
            bus,
            tools,
            store,
            forgotten,
        }
    }

    async fn install_latest(&self) {
        let preview = self.installer.preview("jane/hello", None).await.unwrap();
        self.installer
            .install(&preview.token, Some(&preview.sha256))
            .await
            .unwrap();
        self.wait_for(|s| *s == Status::Running).await;
    }

    async fn wait_for(&self, wanted: impl Fn(&Status) -> bool) -> Status {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(info) = self.runtime.info("hello")
                && wanted(&info.status)
            {
                return info.status;
            }
            assert!(Instant::now() < until, "{:?}", self.runtime.info("hello"));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn version(&self) -> String {
        self.installer.list().await[0].version.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preview_shows_what_it_may_do_and_the_install_runs_exactly_that() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(&with_logo(MANIFEST))).await;
    let home = Home::new(&hub, options()).await;

    let preview = home
        .installer
        .preview("https://github.com/jane/hello", None)
        .await
        .unwrap();
    assert_eq!(preview.id, "hello");
    assert_eq!(preview.version, "0.1.0");
    assert_eq!(preview.repo, "https://github.com/jane/hello");
    assert_eq!(preview.tag, "v0.1.0");
    assert_eq!(preview.group.label, "Custom");
    assert_eq!(preview.sha256, niles_db::hex_digest(COMPONENT));
    assert_eq!(
        preview.logo.as_deref(),
        Some(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="currentColor"/></svg>"#
        ),
        "kept as the sanitiser writes it"
    );
    for line in [
        "Connect to example.com",
        "Connect to any address under example.net",
        "Reach the device at the address you enter under Address, on your home network",
        "Add devices to your home. They appear in rooms, in rules and by voice.",
        "See every device in your home change: lights, doors, motion",
        "Know when the house becomes occupied or empty",
        "Speak through your satellites",
        "Adds a page to the menu: Hello",
        "Adds a card to the Office: Hello now",
    ] {
        assert!(
            preview.permissions.iter().any(|p| p == line),
            "{line}: {:?}",
            preview.permissions
        );
    }
    let tools = preview
        .permissions
        .iter()
        .find(|p| p.starts_with("Offer 16 tools to the assistant: "))
        .expect("the tools line");
    assert!(tools.contains("greet"), "{tools}");
    assert!(preview.cannot.starts_with("It cannot see your conversations"));
    let token = preview
        .settings
        .iter()
        .find(|s| s.key == "token")
        .unwrap();
    assert_eq!(token.kind, "secret");
    assert!(home.runtime.info("hello").is_none(), "nothing runs yet");

    let listed = home
        .installer
        .install(&preview.token, Some(&preview.sha256))
        .await
        .unwrap();
    assert_eq!(listed.version, "0.1.0");
    assert_eq!(listed.source.as_ref().unwrap().tag, "v0.1.0");
    assert!(listed.has_logo);
    home.wait_for(|s| *s == Status::Running).await;
    assert_eq!(
        home.installer.logo("hello").await.unwrap(),
        preview.logo,
        "the sanitised logo is what is kept"
    );

    let again = home
        .installer
        .install(&preview.token, None)
        .await
        .unwrap_err();
    assert!(matches!(again, Refusal::Gone(_)), "a token installs once");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tag_is_installed_from_the_request_or_the_url() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    let preview = home
        .installer
        .preview("jane/hello", Some("v0.1.0"))
        .await
        .unwrap();
    assert_eq!(preview.tag, "v0.1.0");
    let preview = home
        .installer
        .preview("https://github.com/jane/hello/releases/tag/v0.1.0", None)
        .await
        .unwrap();
    assert_eq!(preview.release_url, "https://github.com/jane/hello/releases/tag/v0.1.0");

    let refused = home
        .installer
        .preview("jane/hello", Some("v9.9.9"))
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        Refusal::NotFound("jane/hello has no release tagged v9.9.9.".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_releases_and_no_repo_are_told_apart() {
    let hub = Hub::new().await;
    let home = Home::new(&hub, options()).await;
    let refused = home.installer.preview("jane/hello", None).await.unwrap_err();
    match refused {
        Refusal::NotFound(why) => assert!(why.contains("has no releases yet"), "{why}"),
        other => panic!("{other:?}"),
    }
    let refused = home.installer.preview("jane/private", None).await.unwrap_err();
    match refused {
        Refusal::NotFound(why) => assert!(why.contains("no public repository"), "{why}"),
        other => panic!("{other:?}"),
    }
    let refused = home.installer.preview("not a repo", None).await.unwrap_err();
    assert!(matches!(refused, Refusal::BadRequest(_)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rate_limit_is_said_plainly() {
    let hub = Hub::new().await;
    Mock::given(method("GET"))
        .and(path("/repos/jane/hello/releases/latest"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "1800000000"),
        )
        .mount(&hub.server)
        .await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Upstream(why) => {
            assert!(why.contains("60 requests an hour"), "{why}");
            assert!(why.contains("resets at"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_without_the_plugins_files_is_not_a_plugin() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", vec![asset("niles-integration.toml", MANIFEST)])
        .await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("integration.wasm"), "{why}"),
        other => panic!("{other:?}"),
    }

    let hub = Hub::new().await;
    let mut assets = plugin_assets(&with_logo(MANIFEST));
    assets.retain(|a| a.name != "logo.svg");
    hub.latest("v0.1.0", assets).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("logo.svg"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_assets_are_refused_by_their_size_or_as_they_arrive() {
    let hub = Hub::new().await;
    let mut assets = plugin_assets(MANIFEST);
    assets[0].size = Some(70 * 1024);
    hub.latest("v0.1.0", assets).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("at most 64 KiB"), "{why}"),
        other => panic!("{other:?}"),
    }

    // GitHub says it is small; the bytes say otherwise.
    let hub = Hub::new().await;
    let mut assets = plugin_assets(&with_logo(MANIFEST));
    let logo = assets.iter_mut().find(|a| a.name == "logo.svg").unwrap();
    logo.body = vec![b' '; 40 * 1024];
    logo.size = Some(100);
    logo.digest = None;
    hub.latest("v0.1.0", assets).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("at most 32 KiB"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_is_not_what_github_describes_is_refused() {
    let hub = Hub::new().await;
    let mut assets = plugin_assets(MANIFEST);
    assets[1].digest = Some(format!("sha256:{}", "0".repeat(64)));
    hub.latest("v0.1.0", assets).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("does not match the sha256"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tag_that_is_not_the_manifests_version_is_refused() {
    let hub = Hub::new().await;
    hub.latest("v0.2.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("tagged v0.2.0"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_for_a_newer_niles_is_refused_before_the_component_is_fetched() {
    let hub = Hub::new().await;
    let newer = MANIFEST.replace("min_niles = \"0.2.0\"", "min_niles = \"99.0.0\"");
    let mut assets = plugin_assets(&newer);
    assets[1].size = Some(1 << 40);
    hub.latest("v0.1.0", assets).await;
    let home = Home::new(&hub, options()).await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Invalid(why) => assert!(why.contains("needs Niles 99.0.0"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_install_must_match_what_was_approved() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    let preview = home.installer.preview("jane/hello", None).await.unwrap();
    let refused = home
        .installer
        .install(&preview.token, Some(&"ab".repeat(32)))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, Refusal::Conflict(why) if why.contains("not the")),
        "{refused:?}"
    );
    assert!(home.runtime.info("hello").is_none());
    assert!(home.store.records().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_expires() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(
        &hub,
        Options {
            staging_ttl: Duration::from_millis(50),
            ..options()
        },
    )
    .await;
    let preview = home.installer.preview("jane/hello", None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let refused = home
        .installer
        .install(&preview.token, None)
        .await
        .unwrap_err();
    assert!(matches!(refused, Refusal::Gone(_)), "{refused:?}");
    let refused = home.installer.install("nonsense", None).await.unwrap_err();
    assert!(matches!(refused, Refusal::Gone(_)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn what_would_clash_is_refused() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;

    // Installed already.
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Conflict(why) => assert!(why.contains("already installed"), "{why}"),
        other => panic!("{other:?}"),
    }

    // A tool named like one of Niles's own.
    let home = Home::build(&hub, options(), |i| {
        i.with_reserved_tools(vec!["hello_greet".into()])
    })
    .await;
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Conflict(why) => {
            assert!(why.contains("hello_greet has the name of one of Niles's own"), "{why}")
        }
        other => panic!("{other:?}"),
    }

    // A tool named like another plugin's.
    let home = Home::new(&hub, options()).await;
    home.tools.set("hel", Vec::new(), vec![Arc::new(Named("hello_greet"))]);
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Conflict(why) => assert!(why.contains("one of hel's tools"), "{why}"),
        other => panic!("{other:?}"),
    }

    // Devices under its source, from somewhere else.
    let home = Home::new(&hub, options()).await;
    home.registry.upsert(Device::new(
        DeviceId::parse("x_hello:office/lamp").unwrap(),
        DeviceState::default(),
        DeviceClass::Light,
    ));
    match home.installer.preview("jane/hello", None).await.unwrap_err() {
        Refusal::Conflict(why) => assert!(why.contains("x_hello"), "{why}"),
        other => panic!("{other:?}"),
    }
}

struct Named(&'static str);

#[async_trait]
impl niles_tools::Tool for Named {
    fn descriptor(&self) -> niles_tools::ToolDescriptor {
        niles_tools::ToolDescriptor {
            name: self.0.into(),
            description: "x".into(),
            parameters: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _: serde_json::Value) -> niles_tools::Result<serde_json::Value> {
        Ok(json!(null))
    }
}

/// `jane/hello` at 0.1.0, with 0.2.0 (which also wants button presses),
/// two 0.3.0 pre-releases, and an older 0.0.9 released after it.
async fn released_versions(hub: &Hub) {
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let two = manifest("0.2.0").replace(
        "events = [\"device_state\", \"presence\"]",
        "events = [\"device_state\", \"presence\", \"device_action\"]",
    );
    let releases = vec![
        hub.release("v0.3.0-beta.2", true, plugin_assets(&manifest("0.3.0-beta.2")))
            .await,
        hub.release("v0.3.0-beta.1", true, plugin_assets(&manifest("0.3.0-beta.1")))
            .await,
        hub.release("v0.0.9", false, plugin_assets(&manifest("0.0.9")))
            .await,
        hub.release("v0.2.0", false, plugin_assets(&two)).await,
        hub.release("v0.1.0", false, plugin_assets(MANIFEST)).await,
    ];
    for release in &releases {
        let tag = release["tag_name"].as_str().unwrap();
        hub.answer(
            &format!("/repos/jane/hello/releases/tags/{tag}"),
            200,
            release.clone(),
        )
        .await;
    }
    hub.answer("/repos/jane/hello/releases", 200, json!(releases))
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_release_is_found_with_what_it_would_change() {
    let hub = Hub::new().await;
    released_versions(&hub).await;
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;

    let update = home.installer.check_update("hello").await.unwrap().unwrap();
    assert_eq!(update.version, "0.2.0", "not the pre-release, not the older one");
    assert_eq!(update.tag, "v0.2.0");
    assert_eq!(update.notes, "What is new in v0.2.0.");
    assert_eq!(update.added, vec!["See button presses"]);
    assert!(update.removed.is_empty());
    assert_eq!(
        home.installer.list().await[0].update.as_ref(),
        Some(&update),
        "recorded for the list"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pre_release_is_offered_only_to_a_pre_release() {
    let hub = Hub::new().await;
    released_versions(&hub).await;
    let home = Home::new(&hub, options()).await;
    let preview = home
        .installer
        .preview("jane/hello", Some("v0.3.0-beta.1"))
        .await
        .unwrap();
    home.installer.install(&preview.token, None).await.unwrap();
    let update = home.installer.check_update("hello").await.unwrap().unwrap();
    assert_eq!(update.version, "0.3.0-beta.2");
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_newer_is_no_update() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let only = hub.release("v0.1.0", false, plugin_assets(MANIFEST)).await;
    hub.answer("/repos/jane/hello/releases", 200, json!([only]))
        .await;
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;
    assert_eq!(home.installer.check_update("hello").await.unwrap(), None);
    match home.installer.update_preview("hello").await.unwrap_err() {
        Refusal::Conflict(why) => assert!(why.contains("newest"), "{why}"),
        other => panic!("{other:?}"),
    }
}

async fn updated(hub: &Hub) -> Home {
    released_versions(hub).await;
    let home = Home::new(hub, options()).await;
    home.install_latest().await;
    let preview = home.installer.update_preview("hello").await.unwrap();
    assert_eq!(preview.current_version.as_deref(), Some("0.1.0"));
    assert_eq!(preview.version, "0.2.0");
    assert_eq!(preview.added, vec!["See button presses"]);

    let wrong = home.installer.install(&preview.token, None).await.unwrap_err();
    assert!(matches!(wrong, Refusal::BadRequest(_)), "{wrong:?}");
    let preview = home.installer.update_preview("hello").await.unwrap();
    let listed = home
        .installer
        .update("hello", &preview.token, Some(&preview.sha256))
        .await
        .unwrap();
    assert_eq!(listed.version, "0.2.0");
    assert_eq!(listed.previous_version.as_deref(), Some("0.1.0"));
    assert_eq!(listed.update, None, "this was the update");
    home.wait_for(|s| *s == Status::Running).await;
    home
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_is_previewed_with_its_difference_and_then_runs() {
    let hub = Hub::new().await;
    let home = updated(&hub).await;
    assert_eq!(home.runtime.info("hello").unwrap().version, "0.2.0");
    match home.installer.update_preview("hello").await.unwrap_err() {
        Refusal::Conflict(why) => assert!(why.contains("newest"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_that_fails_soon_after_is_rolled_back() {
    let hub = Hub::new().await;
    let home = updated(&hub).await;
    home.runtime
        .call_tool("hello", "panic", "{}")
        .await
        .unwrap_err();

    let until = Instant::now() + Duration::from_secs(20);
    while home.version().await != "0.1.0" {
        assert!(Instant::now() < until, "still {}", home.version().await);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    home.wait_for(|s| *s == Status::Running).await;
    let listed = &home.installer.list().await[0];
    assert_eq!(listed.previous_version, None, "a rollback happens once");
    let notice = listed.error.as_deref().unwrap();
    assert!(notice.contains("Version 0.2.0 failed"), "{notice}");
    assert!(notice.contains("0.1.0 was put back"), "{notice}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_that_keeps_working_is_kept() {
    let hub = Hub::new().await;
    let home = updated(&hub).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(home.version().await, "0.2.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_can_be_rolled_back_once_by_hand() {
    let hub = Hub::new().await;
    let home = updated(&hub).await;
    let listed = home.installer.roll_back("hello").await.unwrap();
    assert_eq!(listed.version, "0.1.0");
    home.wait_for(|s| *s == Status::Running).await;
    assert_eq!(home.runtime.info("hello").unwrap().version, "0.1.0");
    let again = home.installer.roll_back("hello").await.unwrap_err();
    assert!(matches!(again, Refusal::Conflict(_)), "{again:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_stops_it_and_keeps_everything() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;
    home.runtime
        .call_tool("hello", "remember", r#"{"key":"k","value":"v"}"#)
        .await
        .unwrap();

    let listed = home.installer.disable("hello").await.unwrap();
    assert_eq!(
        (listed.status.as_str(), listed.status_line.as_str()),
        ("disabled", "Off")
    );
    assert!(!listed.enabled);
    assert!(home.runtime.info("hello").is_none(), "not running");
    let lamp = home
        .registry
        .get(&DeviceId::parse("x_hello:office/lamp").unwrap())
        .expect("still in its room");
    assert!(!lamp.available);
    assert!(
        home.store.installed().await.unwrap().is_empty(),
        "not started with Niles"
    );

    home.installer.enable("hello").await.unwrap();
    home.wait_for(|s| *s == Status::Running).await;
    assert_eq!(
        home.runtime
            .call_tool("hello", "recall", r#"{"key":"k"}"#)
            .await
            .unwrap(),
        "v",
        "its storage was kept"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_takes_its_devices_storage_settings_and_secrets() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;
    home.runtime
        .call_tool("hello", "remember", r#"{"key":"k","value":"v"}"#)
        .await
        .unwrap();
    let mut events = home.bus.subscribe();

    home.installer.remove("hello").await.unwrap();

    assert!(home.runtime.info("hello").is_none());
    assert!(
        home.registry
            .list_all()
            .iter()
            .all(|d| d.id.source() != "x_hello"),
        "its devices left the house"
    );
    let mut removed = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::DeviceRemoved { id } = event {
            removed.push(id.to_string());
        }
    }
    removed.sort();
    assert_eq!(
        removed,
        vec![
            "x_hello:office/button",
            "x_hello:office/lamp",
            "x_hello:system/meter"
        ]
    );
    assert!(
        home.tools.descriptors_of("hello").is_empty(),
        "its tools are gone"
    );
    assert!(home.store.records().await.unwrap().is_empty());
    assert_eq!(home.store.get("hello", "k").await.unwrap(), None);
    assert!(home.store.devices("hello").await.unwrap().is_empty());
    assert_eq!(
        *home.forgotten.0.lock().unwrap(),
        vec!["hello"],
        "settings and secrets"
    );
    assert!(home.installer.list().await.is_empty());

    let again = home.installer.remove("hello").await.unwrap_err();
    assert!(matches!(again, Refusal::NotFound(_)), "{again:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_database_nothing_is_installed() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::build(&hub, options(), |i| i.with_database(false)).await;
    let refused = home.installer.preview("jane/hello", None).await.unwrap_err();
    assert!(matches!(refused, Refusal::Unavailable(_)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn declared_secrets_are_listed_under_the_plugins_key() {
    let hub = Hub::new().await;
    hub.latest("v0.1.0", plugin_assets(MANIFEST)).await;
    let home = Home::new(&hub, options()).await;
    home.install_latest().await;
    assert_eq!(
        home.installer.secret_settings().await,
        vec![("plugins.hello.token".to_string(), "Hello: Token".to_string())]
    );
}

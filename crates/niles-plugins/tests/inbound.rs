//! Interface 0.3's accounts and webhooks against a real component:
//! `fixtures/sleepmat.wasm`, built from `fixtures/sleepmat/` (see its
//! `build.sh`). Nothing leaves the machine: the sign-in page is only
//! ever named, never fetched.

use async_trait::async_trait;
use niles_core::{DeviceRegistry, Event, EventBus};
use niles_plugins::{
    HookAnswer, HookRequest, House, KeptSecrets, Limits, MemoryKept, MemoryStore, Runtime, Status,
    SurfaceError,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MANIFEST: &str = include_str!("fixtures/sleepmat/niles-integration.toml");
const COMPONENT: &[u8] = include_bytes!("fixtures/sleepmat.wasm");
const ID: &str = "sleepmat";
const PUBLIC: &str = "https://home.example.dk";

fn quick() -> Limits {
    Limits {
        call: Duration::from_secs(5),
        init: Duration::from_secs(10),
        render: Duration::from_secs(3),
        stop: Duration::from_secs(1),
        webhook: Duration::from_secs(5),
        backoff: vec![Duration::from_millis(50), Duration::from_millis(100)],
        restarts_per_hour: 5,
        init_retry: Duration::from_millis(200),
        tick: Duration::from_millis(10),
    }
}

/// Kept in memory, but a value of `fail` is refused the way an
/// unreachable database would refuse it.
#[derive(Default)]
struct Flaky(MemoryKept);

#[async_trait]
impl KeptSecrets for Flaky {
    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.0.get(key).await
    }
    async fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if value == "fail" {
            return Err("the database is not answering".into());
        }
        self.0.set(key, value).await
    }
    async fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key).await
    }
    async fn keys(&self, prefix: &str) -> Result<Vec<String>, String> {
        self.0.keys(prefix).await
    }
}

struct Home {
    runtime: Runtime,
    bus: EventBus,
    kept: Arc<Flaky>,
    public: Arc<Mutex<Option<String>>>,
}

impl Home {
    async fn new(limits: Limits, public: Option<&str>) -> Self {
        let bus = EventBus::new(1024);
        let kept = Arc::new(Flaky::default());
        let public = Arc::new(Mutex::new(public.map(str::to_string)));
        let read = public.clone();
        let house = House::new(
            Arc::new(DeviceRegistry::new()),
            bus.clone(),
            Arc::new(MemoryStore::new()),
        )
        .with_public_url(Arc::new(move || read.lock().unwrap().clone()))
        .with_kept_secrets(kept.clone());
        let runtime = Runtime::new(house, limits).expect("the runtime starts");
        runtime
            .install(MANIFEST, COMPONENT.to_vec())
            .await
            .expect("the sleep mat installs");
        let home = Self {
            runtime,
            bus,
            kept,
            public,
        };
        home.wait_for(|s| *s == Status::Running).await;
        home
    }

    async fn running() -> Self {
        Self::new(quick(), Some(PUBLIC)).await
    }

    async fn wait_for(&self, wanted: impl Fn(&Status) -> bool) -> Status {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let status = self.runtime.info(ID).expect("it is loaded").status;
            if wanted(&status) {
                return status;
            }
            assert!(Instant::now() < until, "still {status:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn tool(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        self.runtime.call_tool(ID, name, &args.to_string()).await
    }

    async fn token(&self) -> String {
        let url = self.runtime.hook_url(ID).await.expect("it has a webhook");
        url.rsplit('/').next().unwrap().to_string()
    }

    async fn hook(&self, token: &str, method: &str, body: &str) -> HookAnswer {
        self.runtime
            .hook(
                ID,
                token,
                HookRequest {
                    method: method.into(),
                    body: body.as_bytes().to_vec(),
                    ..Default::default()
                },
            )
            .await
    }
}

fn text(answer: &HookAnswer) -> &str {
    std::str::from_utf8(&answer.body).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_built_for_0_3_runs_and_knows_its_webhook_address() {
    let home = Home::running().await;
    let url = home.tool("hook_url", serde_json::json!({})).await.unwrap();
    let token = url
        .strip_prefix("https://home.example.dk/plugins/sleepmat/hook/")
        .unwrap_or_else(|| panic!("{url}"));
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        home.runtime.hook_url(ID).await.as_deref(),
        Some(url.as_str()),
        "the plugin and its panel are told the same address"
    );
    assert_eq!(
        home.runtime.callback_url(ID).await.as_deref(),
        Some("https://home.example.dk/plugins/sleepmat/callback")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_public_address_there_is_no_webhook_and_nothing_to_connect() {
    let home = Home::new(quick(), None).await;
    assert_eq!(
        home.tool("hook_url", serde_json::json!({})).await.unwrap(),
        "none"
    );
    assert_eq!(home.runtime.hook_url(ID).await, None);
    match home.runtime.authorize_url(ID, "st8").await {
        Err(SurfaceError::Unavailable(why)) => assert!(why.contains("public address"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Setting it is enough; nothing has to be installed again.
    *home.public.lock().unwrap() = Some(PUBLIC.into());
    assert!(home.runtime.authorize_url(ID, "st8").await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sign_in_page_is_the_plugins_and_the_way_back_is_niles() {
    let home = Home::running().await;
    let to = home.runtime.authorize_url(ID, "st8").await.unwrap();
    assert_eq!(
        to,
        "https://account.example.com/authorize?redirect_uri=https://home.example.dk/plugins/sleepmat/callback&state=st8"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_page_on_a_host_it_never_declared_is_refused() {
    let home = Home::running().await;
    home.tool("misbehave", serde_json::json!({})).await.unwrap();
    match home.runtime.authorize_url(ID, "st8").await {
        Err(SurfaceError::Plugin(why)) => assert!(why.contains("evil.example.org"), "{why}"),
        other => panic!("the browser must not be sent there: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn coming_back_with_a_code_keeps_the_token_encrypted_and_says_so() {
    let home = Home::running().await;
    let said = home
        .runtime
        .complete_sign_in(
            ID,
            vec![("code".into(), "good".into())],
            Some("mark".into()),
        )
        .await
        .unwrap();
    assert_eq!(said, "Connected for mark");

    let kept = home
        .kept
        .get("plugins.sleepmat.kept.token")
        .await
        .unwrap()
        .expect("kept in the secret store, under the plugin's own prefix");
    let pair: serde_json::Value = serde_json::from_str(&kept).unwrap();
    assert_eq!(pair["refresh"], "r1");
    assert_eq!(
        pair["callback"], "https://home.example.dk/plugins/sleepmat/callback",
        "the code is exchanged with the address it was issued for"
    );
    assert_eq!(
        home.tool("kept", serde_json::json!({})).await.unwrap(),
        kept
    );

    match home
        .runtime
        .complete_sign_in(ID, vec![("error".into(), "access_denied".into())], None)
        .await
    {
        Err(SurfaceError::Plugin(why)) => assert_eq!(why, "The mat was not given access."),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bed_event_on_the_webhook_is_an_action_rules_can_start_on() {
    let home = Home::running().await;
    let token = home.token().await;
    let mut events = home.bus.subscribe();

    let answer = home.hook(&token, "POST", "userid=1&appli=50").await;
    assert_eq!(answer.status, 200);
    assert_eq!(text(&answer), "ok");
    assert_eq!(answer.content_type.as_deref(), Some("text/plain"));

    let action = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::DeviceAction { id, action }) = events.recv().await {
                return (id.to_string(), action);
            }
        }
    })
    .await
    .expect("an action");
    assert_eq!(action, ("x_sleepmat:bedroom/bed".into(), "bed_in".into()));

    let last = home.runtime.last_hook(ID).expect("the panel can say when");
    assert_eq!(last.status, 200);
    let logged = home.runtime.log(ID);
    let line = logged
        .iter()
        .rev()
        .find(|l| l.message.starts_with("hook "))
        .expect("a line for the request");
    assert!(
        line.message.starts_with("hook POST 17 B → 200"),
        "{}",
        line.message
    );
    assert!(
        logged
            .iter()
            .all(|l| !l.message.contains(&token) && !l.message.contains("userid")),
        "neither the token nor what the request said is logged"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_right_token_and_a_get_or_post_reach_the_plugin() {
    let home = Home::running().await;
    let token = home.token().await;

    assert_eq!(
        home.hook(&"0".repeat(64), "POST", "appli=50").await.status,
        404
    );
    assert_eq!(home.hook("", "POST", "appli=50").await.status, 404);
    assert_eq!(home.hook(&token, "PUT", "appli=50").await.status, 405);

    let head = home.hook(&token, "HEAD", "").await;
    assert_eq!(head.status, 200, "a service checking the address");
    assert!(head.body.is_empty());
    assert!(
        home.runtime.last_hook(ID).is_none(),
        "answered by Niles alone"
    );

    let challenge = home
        .runtime
        .hook(
            ID,
            &token,
            HookRequest {
                method: "GET".into(),
                query: vec![("challenge".into(), "abc123".into())],
                ..Default::default()
            },
        )
        .await;
    assert_eq!(challenge.status, 200);
    assert_eq!(
        text(&challenge),
        "abc123",
        "the plugin's own answer goes back"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn while_it_is_off_the_service_is_told_to_try_again_and_once_removed_it_is_gone() {
    let home = Home::running().await;
    let token = home.token().await;

    home.runtime.disable(ID).await.unwrap();
    assert_eq!(
        home.hook(&token, "POST", "appli=50").await.status,
        503,
        "never acknowledged while nobody can act on it"
    );
    assert_eq!(home.hook(&token, "HEAD", "").await.status, 200);

    home.runtime.enable(ID).await.unwrap();
    home.wait_for(|s| *s == Status::Running).await;
    assert_eq!(
        home.token().await,
        token,
        "the address outlives turning it off"
    );
    assert_eq!(home.hook(&token, "POST", "appli=51").await.status, 200);

    home.runtime.remove(ID).await.unwrap();
    assert_eq!(home.hook(&token, "POST", "appli=51").await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_address_retires_the_old_one_and_restarts_the_plugin() {
    let home = Home::running().await;
    let old = home.token().await;
    home.runtime.rotate_hook(ID).await.unwrap();
    home.wait_for(|s| *s == Status::Running).await;
    let new = home.token().await;
    assert_ne!(old, new);
    assert_eq!(home.hook(&old, "POST", "appli=50").await.status, 404);
    assert_eq!(home.hook(&new, "POST", "appli=50").await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_webhook_that_misses_its_deadline_is_503_and_the_plugin_comes_back() {
    let home = Home::new(
        Limits {
            webhook: Duration::from_millis(300),
            ..quick()
        },
        Some(PUBLIC),
    )
    .await;
    let token = home.token().await;
    let started = Instant::now();
    assert_eq!(home.hook(&token, "POST", "spin").await.status, 503);
    assert!(started.elapsed() < Duration::from_secs(3));
    home.wait_for(|s| *s == Status::Running).await;
    assert_eq!(home.hook(&token, "POST", "appli=50").await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_past_the_limit_is_429_without_calling_the_plugin() {
    let home = Home::running().await;
    let token = home.token().await;
    for n in 0..20 {
        assert_eq!(
            home.hook(&token, "POST", "appli=50").await.status,
            200,
            "{n}"
        );
    }
    assert_eq!(home.hook(&token, "POST", "appli=50").await.status, 429);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_that_cannot_be_kept_leaves_the_previous_one() {
    let home = Home::running().await;
    let keep = |value: &str| serde_json::json!({ "key": "token", "value": value });
    home.tool("keep", keep("v1")).await.unwrap();
    let refused = home.tool("keep", keep("fail")).await.unwrap_err();
    assert!(refused.contains("Unavailable"), "{refused}");
    assert_eq!(
        home.tool("kept", serde_json::json!({})).await.unwrap(),
        "v1"
    );

    home.tool("forget", serde_json::json!({})).await.unwrap();
    assert_eq!(
        home.tool("kept", serde_json::json!({})).await.unwrap(),
        "none"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn kept_secrets_are_few_small_and_plainly_named() {
    let home = Home::running().await;
    let keep = |key: &str, value: &str| serde_json::json!({ "key": key, "value": value });
    let bad = home
        .tool("keep", keep("Refresh-Token", "x"))
        .await
        .unwrap_err();
    assert!(bad.contains("Invalid"), "{bad}");
    let big = home
        .tool("keep", keep("big", &"x".repeat(4 * 1024 + 1)))
        .await
        .unwrap_err();
    assert!(big.contains("Limit"), "{big}");
    for n in 0..16 {
        home.tool("keep", keep(&format!("k{n}"), "x"))
            .await
            .unwrap();
    }
    let seventeenth = home.tool("keep", keep("k16", "x")).await.unwrap_err();
    assert!(seventeenth.contains("Limit"), "{seventeenth}");
    home.tool("keep", keep("k3", "replaced"))
        .await
        .expect("replacing one of the sixteen is not a seventeenth");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_0_3_component_under_an_older_manifest_is_refused() {
    let home = Home::running().await;
    let older = MANIFEST
        .replace("id = \"sleepmat\"", "id = \"oldmat\"")
        .replace("interface = \"0.3\"", "interface = \"0.2\"")
        .replace("callback = true\n", "")
        .replace("webhook = true\n", "");
    let refused = home
        .runtime
        .install(&older, COMPONENT.to_vec())
        .await
        .expect_err("the manifest and the component disagree");
    assert!(refused.to_string().contains("0.3"), "{refused}");
}

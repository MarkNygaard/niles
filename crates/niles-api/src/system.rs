//! Restarting Niles from the app, and saying when a restart waits.
//!
//! What waits is worked out by comparing what this process started with
//! ([`niles_config::Booted`]) with the store now, never by remembering
//! writes: a change from the app, by voice or over the API counts the
//! same, changing it back clears it, and every session asking gets the
//! same answer.
//!
//! A restart is an orderly stop and an exit with code 0, which whatever
//! runs Niles answers by starting it again — see the binary's
//! `shutdown.rs` for the order. This module only holds the handle: the
//! fingerprint, the boot id, who may be told, and the flag the serve
//! loop waits on.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use niles_config::{Booted, ConfigStore};
use niles_core::{Event, EventBus};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

/// This start of Niles: who it is while it runs, and how the app knows
/// the next one is up.
#[derive(Debug, Clone)]
pub struct Boot {
    id: String,
    started_at: DateTime<Utc>,
}

impl Boot {
    /// A fresh random id and the moment of starting.
    pub fn start() -> Self {
        let bytes: [u8; 16] = std::array::from_fn(|_| rand::random());
        let id = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Self {
            id,
            started_at: Utc::now(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
}

/// Whatever starts Niles again when it exits: Kubernetes, found from the
/// environment, or one declared by name.
#[non_exhaustive]
pub enum Supervisor {
    /// `KUBERNETES_SERVICE_HOST` is set, so this pod restarts its
    /// container when it exits.
    Kubernetes,
    /// `NILES_SUPERVISOR` named it (`docker`, `systemd`, …) and whatever
    /// it is restarts Niles after a clean exit.
    Declared(String),
}

impl Supervisor {
    /// What runs this Niles, when the environment says.
    pub fn detect() -> Option<Self> {
        Self::from_env(|name| std::env::var(name).ok())
    }

    fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if var("KUBERNETES_SERVICE_HOST").is_some_and(|value| !value.trim().is_empty()) {
            return Some(Self::Kubernetes);
        }
        var("NILES_SUPERVISOR")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| Self::Declared(value.to_ascii_lowercase()))
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Kubernetes => "kubernetes",
            Self::Declared(name) => name,
        }
    }
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Supervisor").field(&self.name()).finish()
    }
}

/// What a restart would cut short now. Answered by the binary, which
/// runs all of it.
pub trait UnderWay: Send + Sync {
    fn under_way(&self) -> CutShort;
}

/// The things a restart would cut short, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CutShort {
    /// Background jobs the assistant is doing: artifacts being written,
    /// pages and changes being made.
    pub jobs: usize,
    /// Wake-up ramps under way. A restart pauses them; they carry on
    /// when Niles is back, since what is under way is kept.
    pub wake_ups: usize,
    /// Approvals waiting for somebody's yes. They lapse.
    pub approvals: usize,
}

/// One section a restart would apply a change to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The top-level config section, e.g. `mqtt`.
    pub what: String,
    /// The name the Settings page gives it, e.g. `Zigbee & MQTT`.
    pub label: String,
    /// When it started waiting.
    pub since: DateTime<Utc>,
}

/// What asking for a restart answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// The stop is on its way.
    Started,
    /// Somebody already asked; the first one stands.
    AlreadyUnderWay,
}

/// Nobody runs this Niles, so a restart from here would only stop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotSupervised;

/// The handle the API answers from: the fingerprint of what this process
/// started with, the boot id, and the flag the serve loop waits on.
pub struct Restart {
    boot: Boot,
    booted: Booted,
    store: Arc<ConfigStore>,
    bus: EventBus,
    supervisor: Option<Supervisor>,
    under_way: Option<Arc<dyn UnderWay>>,
    /// What waited at the last check, and since when. Replaced whole on
    /// each check.
    since: std::sync::Mutex<BTreeMap<String, DateTime<Utc>>>,
    /// Flipped once, by the first request. The serve loop waits on it.
    requested: tokio::sync::watch::Sender<bool>,
}

impl Restart {
    pub fn new(
        boot: Boot,
        booted: Booted,
        store: Arc<ConfigStore>,
        bus: EventBus,
        supervisor: Option<Supervisor>,
    ) -> Self {
        Self {
            boot,
            booted,
            store,
            bus,
            supervisor,
            under_way: None,
            since: std::sync::Mutex::new(BTreeMap::new()),
            requested: tokio::sync::watch::channel(false).0,
        }
    }

    pub fn with_under_way(mut self, under_way: Arc<dyn UnderWay>) -> Self {
        self.under_way = Some(under_way);
        self
    }

    pub fn boot(&self) -> &Boot {
        &self.boot
    }

    pub fn supervisor(&self) -> Option<&Supervisor> {
        self.supervisor.as_ref()
    }

    /// Whether the overrides a restart would apply outlive the process.
    pub fn is_persistent(&self) -> bool {
        self.store.is_persistent()
    }

    /// What waits for a restart now, re-checking against the store. When
    /// the set changed since the last check, the house hears of it.
    pub fn pending(&self) -> Vec<Pending> {
        // Held across the whole computation so two concurrent checks
        // can't publish out of order.
        let mut since = self.since.lock().unwrap_or_else(|e| e.into_inner());
        let (config, table) = self.store.current_and_table();
        let differs = self.booted.differs(&table, &config);
        let at = Utc::now();
        let next: BTreeMap<String, DateTime<Utc>> = differs
            .iter()
            .map(|what| {
                let when = since.get(what).copied().unwrap_or(at);
                (what.clone(), when)
            })
            .collect();
        let changed = !next.keys().eq(since.keys());
        let pending: Vec<Pending> = differs
            .iter()
            .filter_map(|what| {
                Some(Pending {
                    label: niles_config::section_label(what)
                        .unwrap_or(what)
                        .to_string(),
                    since: next.get(what).copied()?,
                    what: what.clone(),
                })
            })
            .collect();
        *since = next;
        drop(since);
        if changed {
            self.bus.publish(Event::RestartPendingChanged);
        }
        pending
    }

    /// Re-checks on every config and secret change, so open apps hear of
    /// a change made anywhere.
    pub fn watch(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut config_changed = self.store.subscribe();
            let mut secrets_changed = niles_config::secrets::subscribe();
            loop {
                tokio::select! {
                    // A senders' going means nothing more can change;
                    // there is nothing left to watch for.
                    changed = config_changed.changed() => if changed.is_err() {
                        return;
                    },
                    changed = secrets_changed.changed() => if changed.is_err() {
                        return;
                    },
                }
                // The result is published on the bus when the set
                // changed; nobody here reads the list itself.
                let _ = self.pending();
            }
        })
    }

    pub fn cut_short(&self) -> CutShort {
        self.under_way
            .as_ref()
            .map_or_else(CutShort::default, |u| u.under_way())
    }

    /// Ask for a restart. The first ask wins; a later one learns it did.
    pub fn request(&self) -> std::result::Result<Requested, NotSupervised> {
        let Some(supervisor) = &self.supervisor else {
            return Err(NotSupervised);
        };
        if self.requested.send_replace(true) {
            return Ok(Requested::AlreadyUnderWay);
        }
        self.bus.publish(Event::RestartStarted {
            boot_id: self.boot.id.clone(),
        });
        tracing::info!(
            "[restart] asked for in the app; stopping so {} starts Niles again",
            supervisor.name()
        );
        Ok(Requested::Started)
    }

    /// A receiver that resolves once a restart is asked for. The serve
    /// loop waits on this.
    pub fn requested(&self) -> tokio::sync::watch::Receiver<bool> {
        self.requested.subscribe()
    }

    pub fn is_requested(&self) -> bool {
        *self.requested.borrow()
    }
}

#[derive(Serialize)]
struct StatusView {
    boot: BootView,
    pending: Vec<PendingView>,
    supervisor: Option<String>,
    persistent: bool,
    restarting: bool,
    cut_short: CutShort,
}

#[derive(Serialize)]
struct BootView {
    id: String,
    started_at: String,
}

#[derive(Serialize)]
struct PendingView {
    what: String,
    label: String,
    since: String,
}

fn boot_view(boot: &Boot) -> BootView {
    BootView {
        id: boot.id.clone(),
        started_at: boot.started_at.to_rfc3339(),
    }
}

/// `GET /system/restart` — what waits for a restart, this start's boot
/// id, and what a restart would cut short.
pub async fn status(State(state): State<AppState>) -> Response {
    let Some(restart) = &state.restart else {
        return unsupported();
    };
    let pending = restart
        .pending()
        .iter()
        .map(|p| PendingView {
            what: p.what.clone(),
            label: p.label.clone(),
            since: p.since.to_rfc3339(),
        })
        .collect();
    Json(StatusView {
        boot: boot_view(restart.boot()),
        pending,
        supervisor: restart.supervisor().map(|s| s.name().to_string()),
        persistent: restart.is_persistent(),
        restarting: restart.is_requested(),
        cut_short: restart.cut_short(),
    })
    .into_response()
}

/// `POST /system/restart` — ask Niles to stop, so whatever runs it
/// starts it again.
pub async fn restart(State(state): State<AppState>) -> Response {
    let Some(restart) = &state.restart else {
        return unsupported();
    };
    match restart.request() {
        Ok(_) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "boot": boot_view(restart.boot()) })),
        )
            .into_response(),
        Err(NotSupervised) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Niles isn't running under anything that would start it \
                          again, so restarting it from here would only stop it. \
                          Restart it where it runs."
            })),
        )
            .into_response(),
    }
}

/// Subcommands that don't build a store still serve the API, so this is
/// a missing capability rather than an error.
fn unsupported() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "this Niles was started without a config store, so it cannot \
                      say what waits for a restart or restart itself"
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    struct FakeUnderWay(CutShort);

    impl UnderWay for FakeUnderWay {
        fn under_way(&self) -> CutShort {
            self.0
        }
    }

    /// A store and the restart handle over it, with a fake for what a
    /// restart would cut short.
    fn fixture(
        supervisor: Option<Supervisor>,
    ) -> (axum::Router, Arc<ConfigStore>, Arc<Restart>, EventBus) {
        let store = Arc::new(
            ConfigStore::from_str_in_memory(&crate::config_tests::base_toml())
                .expect("fixture is valid"),
        );
        let bus = EventBus::default();
        let (config, table) = store.current_and_table();
        let mut restart = Restart::new(
            Boot::start(),
            Booted::new(&table, &config),
            store.clone(),
            bus.clone(),
            supervisor,
        );
        restart = restart.with_under_way(Arc::new(FakeUnderWay(CutShort {
            jobs: 2,
            wake_ups: 1,
            approvals: 3,
        })));
        let restart = Arc::new(restart);
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(crate::config_tests::NoopPublisher)
                as Arc<dyn crate::publish::DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            bus.clone(),
        )
        .with_config_store(Some(store.clone()))
        .with_restart(Some(restart.clone()));
        (crate::server::router(state), store, restart, bus)
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    fn patch(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("PATCH")
            .uri("/config")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn nothing_waits_at_first() {
        let (app, _, _, _) = fixture(None);
        let (status, body) = send(&app, get("/system/restart")).await;
        assert_eq!(status, StatusCode::OK);
        // Other tests load the global secrets map concurrently, so this
        // only asserts on the sections the fixture touches.
        assert!(
            body["pending"]
                .as_array()
                .expect("a list")
                .iter()
                .all(|p| p["what"] != "api"),
            "the api section matched what Niles started with"
        );
        let id = body["boot"]["id"].as_str().expect("a boot id");
        assert_eq!(id.len(), 32, "sixteen bytes as lowercase hex");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(body["supervisor"], serde_json::Value::Null);
        assert_eq!(
            body["cut_short"],
            serde_json::json!({ "jobs": 2, "wake_ups": 1, "approvals": 3 }),
            "echoed from the binary's answer"
        );
        assert_eq!(body["persistent"], false, "the in-memory fixture store");
        assert_eq!(body["restarting"], false);
    }

    #[tokio::test]
    async fn a_boot_change_waits_with_its_label_and_since() {
        let (app, _, _, _) = fixture(None);
        send(
            &app,
            patch(serde_json::json!({"api": {"bind_address": "0.0.0.0:9090"}})),
        )
        .await;
        let (_, body) = send(&app, get("/system/restart")).await;
        let api = body["pending"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|p| p["what"] == "api")
            .expect("the api section waits");
        assert_eq!(api["label"], "API address");
        assert!(
            chrono::DateTime::parse_from_rfc3339(api["since"].as_str().expect("a timestamp"))
                .is_ok(),
            "since parses as RFC 3339"
        );
    }

    #[tokio::test]
    async fn since_holds_while_it_waits() {
        let (_, _, restart, _) = fixture(None);
        let store = restart.store.clone();
        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:9090\"").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        let first = restart.pending();
        let second = restart.pending();
        let one = first.iter().find(|p| p.what == "api").expect("api waits");
        let two = second.iter().find(|p| p.what == "api").expect("api waits");
        assert_eq!(one.since, two.since, "waiting does not move the start");
    }

    #[tokio::test]
    async fn a_change_by_voice_waits_too() {
        let (_, _, restart, _) = fixture(None);
        let store = restart.store.clone();
        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:9090\"").unwrap(),
                niles_config::ChangeSource::Voice,
            )
            .await
            .expect("voice may change a boot section that is not auth");
        let pending = restart.pending();
        assert!(
            pending.iter().any(|p| p.what == "api"),
            "where a change came from does not matter"
        );
    }

    #[tokio::test]
    async fn changing_it_back_clears_it() {
        let (_, _, restart, _) = fixture(None);
        let store = restart.store.clone();
        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:9090\"").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:8080\"").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert!(!restart.pending().iter().any(|p| p.what == "api"));
    }

    #[tokio::test]
    async fn a_hot_change_never_waits() {
        let (_, _, restart, _) = fixture(None);
        let store = restart.store.clone();
        store
            .apply(
                &toml::from_str("[lighting]\ndaytime_brightness = 85").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert!(!restart.pending().iter().any(|p| p.what == "lighting"));
    }

    #[tokio::test]
    async fn appearing_and_clearing_are_published() {
        let (_, _, restart, bus) = fixture(None);
        let mut rx = bus.subscribe();
        let store = restart.store.clone();
        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:9090\"").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        restart.pending();
        expect_pending_changed(&mut rx).await;

        store
            .apply(
                &toml::from_str("[api]\nbind_address = \"0.0.0.0:8080\"").unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        restart.pending();
        expect_pending_changed(&mut rx).await;
    }

    async fn expect_pending_changed(rx: &mut tokio::sync::broadcast::Receiver<Event>) {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("an event is published")
            .expect("the bus is up");
        assert!(
            matches!(event, Event::RestartPendingChanged),
            "the set changed, the house hears: {event:?}"
        );
    }

    #[tokio::test]
    async fn restarting_without_a_supervisor_is_refused() {
        let (app, _, restart, _) = fixture(None);
        let req = Request::builder()
            .method("POST")
            .uri("/system/restart")
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body["error"]
                .as_str()
                .expect("an error")
                .contains("only stop it")
        );
        assert!(!restart.is_requested(), "nothing was asked for");
    }

    #[tokio::test]
    async fn restarting_under_kubernetes_is_accepted_once() {
        let (app, _, restart, bus) = fixture(Some(Supervisor::Kubernetes));
        let mut rx = bus.subscribe();
        let req = Request::builder()
            .method("POST")
            .uri("/system/restart")
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["boot"]["id"], restart.boot().id());

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("an event is published")
            .expect("the bus is up");
        let Event::RestartStarted { boot_id } = event else {
            panic!("restart_started carries the boot id that is going away: {event:?}");
        };
        assert_eq!(boot_id, restart.boot().id());

        assert!(
            *restart.requested().borrow(),
            "the serve loop's receiver sees the request"
        );
        assert_eq!(
            restart.request(),
            Ok(Requested::AlreadyUnderWay),
            "a second ask learns the first one stands"
        );
        // No second RestartStarted: the first one is the one everybody
        // waits on, and a duplicate would make a second waiting screen.
        let again = rx.try_recv();
        assert!(
            !matches!(again, Ok(Event::RestartStarted { .. })),
            "no second restart_started"
        );
    }

    #[tokio::test]
    async fn get_says_a_restart_is_under_way() {
        let (app, _, restart, _) = fixture(Some(Supervisor::Declared("docker".into())));
        assert_eq!(restart.supervisor().map(Supervisor::name), Some("docker"));
        let req = Request::builder()
            .method("POST")
            .uri("/system/restart")
            .body(Body::empty())
            .unwrap();
        let (status, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (_, body) = send(&app, get("/system/restart")).await;
        assert_eq!(body["restarting"], true);
    }

    #[tokio::test]
    async fn without_the_capability_it_says_so() {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(crate::config_tests::NoopPublisher)
                as Arc<dyn crate::publish::DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            EventBus::default(),
        );
        let app = crate::server::router(state);
        let (status, body) = send(&app, get("/system/restart")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(
            body["error"]
                .as_str()
                .expect("an error")
                .contains("config store")
        );
        let req = Request::builder()
            .method("POST")
            .uri("/system/restart")
            .body(Body::empty())
            .unwrap();
        let (status, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn the_supervisor_is_kubernetes_when_the_pod_says_so() {
        let found = Supervisor::from_env(|name| {
            (name == "KUBERNETES_SERVICE_HOST").then(|| "10.0.0.1".to_string())
        });
        assert!(matches!(found, Some(Supervisor::Kubernetes)));
    }

    #[test]
    fn a_declared_supervisor_is_named() {
        let found =
            Supervisor::from_env(|name| (name == "NILES_SUPERVISOR").then(|| "Docker".to_string()));
        assert!(
            matches!(found, Some(Supervisor::Declared(ref name)) if name == "docker"),
            "declared names are lowercased: {found:?}"
        );
    }

    #[test]
    fn nothing_declared_means_none() {
        for value in ["", "   "] {
            let found = Supervisor::from_env(|name| {
                (name == "NILES_SUPERVISOR").then(|| value.to_string())
            });
            assert!(found.is_none(), "{value:?} names nothing: {found:?}");
        }
        assert!(Supervisor::from_env(|_| None).is_none());
    }

    #[test]
    fn kubernetes_wins_over_a_declared_one() {
        let found = Supervisor::from_env(|name| match name {
            "KUBERNETES_SERVICE_HOST" => Some("10.0.0.1".to_string()),
            "NILES_SUPERVISOR" => Some("docker".to_string()),
            _ => None,
        });
        assert!(matches!(found, Some(Supervisor::Kubernetes)));
    }
}

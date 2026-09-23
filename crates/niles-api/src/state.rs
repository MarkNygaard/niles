//! Shared state plumbed through axum handlers.

use crate::publish::DevicePublisher;
use niles_core::{DeviceRegistry, EventBus};
use niles_mqtt::CommandRouter;
use niles_notifications::NotificationCenter;
use niles_scheduler::ManualModeTracker;
use std::sync::Arc;

/// State required to verify and route Linear webhooks.
pub struct LinearWebhookState {
    pub secret: Vec<u8>,
    pub team: String,
    pub notify_room: Option<String>,
    pub center: Arc<NotificationCenter>,
}

/// State shared with every request. `Clone` is cheap (just bumps
/// reference counts).
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DeviceRegistry>,
    pub publisher: Arc<dyn DevicePublisher>,
    /// Turns a desired state into the topic and payload that device's
    /// source understands — Z2M and WLED speak different MQTT.
    pub router: Arc<CommandRouter>,
    pub event_bus: EventBus,
    pub linear_webhook: Option<Arc<LinearWebhookState>>,
    /// Absent for subcommands that serve the device API without a config
    /// store; the `/config` routes report that rather than 500ing.
    pub config: Option<Arc<niles_config::ConfigStore>>,
    /// Recent log lines, when the binary installed a buffer. Absent for
    /// subcommands that don't, and `/logs` says so rather than 500ing.
    pub logs: Option<crate::logs::LogBuffer>,
    /// Sign-in attempts in flight. In memory on purpose: an attempt is
    /// worth ten minutes, and a restart mid-sign-in costs one retry.
    pub attempts: Arc<crate::auth::flow::Attempts>,
    /// The operator's token, for callers that are not browsers. Not
    /// subject to the allowlist, because it is not a person.
    pub api_token: Option<Arc<String>>,
    /// Where credentials typed into the app are kept. Absent when
    /// there is no database or no encryption key, in which case
    /// secrets come from the environment and cannot be changed here.
    pub secrets: Option<Arc<niles_db::PostgresSecrets>>,
    /// The tado connection, when presence names it. Present so the app
    /// can get it authorised: the device flow needs a person with a
    /// browser, which a service does not have.
    pub tado: Option<Arc<niles_presence::TadoSource>>,
    /// The enrolled voices, when recognition is running. Absent
    /// otherwise, and `/voices` says so rather than 500ing.
    pub voices: Option<Arc<dyn niles_recognition::VoiceRoster>>,
    /// The kept wake audio, when a database is configured.
    pub captures: Option<Arc<niles_db::PostgresCaptures>>,
    /// Which lights the lighting curve must leave alone.
    ///
    /// Absent when nothing is driving a curve — `niles api` serves the
    /// device routes with no scheduler behind them, and there is then
    /// nothing to be exempt from.
    pub manual_mode: Option<Arc<ManualModeTracker>>,
    /// The saved scenes, when this instance has a store for them.
    pub scenes: Option<Arc<niles_scheduler::SceneStore>>,
}

impl AppState {
    pub fn new(
        registry: Arc<DeviceRegistry>,
        publisher: Arc<dyn DevicePublisher>,
        router: Arc<CommandRouter>,
        event_bus: EventBus,
    ) -> Self {
        Self {
            registry,
            publisher,
            router,
            event_bus,
            linear_webhook: None,
            config: None,
            logs: None,
            attempts: Arc::new(crate::auth::flow::Attempts::new()),
            api_token: None,
            manual_mode: None,
            scenes: None,
            tado: None,
            voices: None,
            captures: None,
            secrets: None,
        }
    }

    /// The encrypted secret store, so Settings can fill it in.
    pub fn with_secrets(mut self, secrets: Option<Arc<niles_db::PostgresSecrets>>) -> Self {
        self.secrets = secrets;
        self
    }

    /// The tado source, so Settings can walk somebody through
    /// authorising it.
    pub fn with_tado(mut self, tado: Option<Arc<niles_presence::TadoSource>>) -> Self {
        self.tado = tado;
        self
    }

    /// The kept wake audio, for getting it back out.
    pub fn with_captures(mut self, captures: Option<Arc<niles_db::PostgresCaptures>>) -> Self {
        self.captures = captures;
        self
    }

    /// The enrolled voices, when recognition is running.
    pub fn with_voices(mut self, voices: Option<Arc<dyn niles_recognition::VoiceRoster>>) -> Self {
        self.voices = voices;
        self
    }

    /// The manual-mode flags the lighting curve consults before it
    /// touches a light.
    ///
    /// Optional because the API also runs from `niles api`, which has
    /// no curve driving anything — there is nothing there to be exempt
    /// from. When it is absent, commands simply are not flagged.
    /// The saved scenes, so the dashboard can list and apply them.
    pub fn with_scenes(mut self, scenes: Option<Arc<niles_scheduler::SceneStore>>) -> Self {
        self.scenes = scenes;
        self
    }

    pub fn with_manual_mode(mut self, tracker: Option<Arc<ManualModeTracker>>) -> Self {
        self.manual_mode = tracker;
        self
    }

    /// The bearer token that stands in for a session, when one is
    /// configured. Empty is treated as absent: a placeholder nobody
    /// filled in must not become a password of "".
    pub fn with_api_token(mut self, token: Option<String>) -> Self {
        self.api_token = token.filter(|t| !t.trim().is_empty()).map(Arc::new);
        self
    }

    pub fn with_logs(mut self, logs: Option<crate::logs::LogBuffer>) -> Self {
        self.logs = logs;
        self
    }

    pub fn with_config_store(mut self, store: Option<Arc<niles_config::ConfigStore>>) -> Self {
        self.config = store;
        self
    }

    pub fn with_linear_webhook(mut self, w: Option<Arc<LinearWebhookState>>) -> Self {
        self.linear_webhook = w;
        self
    }
}

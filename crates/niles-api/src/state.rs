//! Shared state plumbed through axum handlers.

use crate::publish::DevicePublisher;
use niles_core::{DeviceRegistry, EventBus};
use niles_mqtt::CommandRouter;
use niles_notifications::NotificationCenter;
use niles_scheduler::ManualModeTracker;
use std::sync::Arc;

/// State required to verify and route Linear webhooks.
pub struct LinearWebhookState {
    /// The signing secret Linear shows when the webhook is made. Read on
    /// every request, so saving it in the app takes hold at once; none
    /// means there is no webhook, and the route answers as if absent.
    pub secret: Box<dyn Fn() -> Option<String> + Send + Sync>,
    /// Where what Linear says goes: an announcement for the house, since
    /// nothing says which room the person who filed it is in.
    pub center: Arc<NotificationCenter>,
}

impl LinearWebhookState {
    /// The webhook as the app sets it up: its secret sealed in the store
    /// under [`crate::webhook::LINEAR_WEBHOOK_SECRET`].
    pub fn new(center: Arc<NotificationCenter>) -> Self {
        Self {
            secret: Box::new(|| niles_config::secrets::get(crate::webhook::LINEAR_WEBHOOK_SECRET)),
            center,
        }
    }
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
    /// Where credentials typed into the app are kept. Absent when
    /// there is no database or no encryption key, in which case
    /// secrets come from the environment and cannot be changed here.
    pub secrets: Option<Arc<dyn crate::secrets::SecretStore>>,
    /// The tado connection, when presence names it. Present so the app
    /// can get it authorised: the device flow needs a person with a
    /// browser, which a service does not have.
    pub tado: Option<Arc<niles_presence::TadoSource>>,
    /// The enrolled voices, when recognition is running. Absent
    /// otherwise, and `/voices` says so rather than 500ing.
    pub voices: Option<Arc<dyn niles_recognition::VoiceRoster>>,
    /// The kept wake audio, when a database is configured.
    pub captures: Option<Arc<niles_db::PostgresCaptures>>,
    /// The UniFi console, when presence names one.
    ///
    /// Here for one question the console alone can answer: which device
    /// is making *this* request. That is how a phone is paired to a
    /// person without anybody typing a MAC address.
    pub unifi: Option<Arc<niles_presence::UnifiSource>>,
    /// Which lights the lighting curve must leave alone.
    ///
    /// Absent when nothing is driving a curve — `niles api` serves the
    /// device routes with no scheduler behind them, and there is then
    /// nothing to be exempt from.
    pub manual_mode: Option<Arc<ManualModeTracker>>,
    /// The saved scenes, when this instance has a store for them.
    pub scenes: Option<Arc<niles_scheduler::SceneStore>>,
    /// The shopping list, shared with the voice tools.
    pub groceries: Option<Arc<niles_groceries::GroceryStore>>,
    /// The household's automations, shared with the voice tools and
    /// the engine that runs them.
    pub automations: Option<Arc<niles_automations::AutomationStore>>,
    /// What each rule did lately, as the engine noted it.
    pub automation_runs: Option<Arc<niles_automations::RunLog>>,
    /// The skills Niles has made, shared with the voice tools.
    pub skills: Option<Arc<niles_skills::SkillStore>>,
    /// What Niles remembers between conversations, shared with the
    /// voice tools and the background review that writes to it.
    pub memory: Option<Arc<niles_memory::MemoryStore>>,
    /// Each person's calendars. Absent without a database and its
    /// secrets key: a refresh token is not kept anywhere unsealed.
    pub calendars: Option<Arc<niles_calendar::Calendars>>,
    /// Calendar sign-ins in flight, apart from the GitHub ones.
    pub calendar_attempts: Arc<crate::auth::flow::Attempts>,
    /// ChatGPT sign-ins started from the app and not yet finished.
    pub chatgpt_logins: Arc<crate::chatgpt::Logins>,
    /// What answers the app's chat, when the binary wired one up.
    pub chat: Option<Arc<dyn crate::chat::Chat>>,
    /// What waits for each person's yes, and what they decided.
    /// Answered by the binary, beside the chat whose cards they are.
    pub approvals: Option<Arc<dyn crate::approvals::Approvals>>,
    /// The rooms' music, for the room cards. Answered by the binary.
    pub music: Option<Arc<dyn crate::music::Music>>,
    /// The nemlig.com session. Present whether or not the integration is
    /// switched on — that is read from the config per request.
    pub nemlig: Option<Arc<niles_nemlig::NemligClient>>,
    /// The third-party plugins, when the binary runs them.
    pub plugins: Option<Arc<dyn crate::plugins::Plugins>>,
    /// The remote MCP servers whose tools the model is offered.
    pub tool_servers: Option<Arc<dyn crate::tool_servers::ToolServers>>,
    /// Each person's Strava and the workouts it brings. Absent without
    /// a database and its secrets key: an app secret and its tokens are
    /// not kept anywhere unsealed.
    pub strava: Option<Arc<niles_training::Strava>>,
    /// Strava sign-ins in flight, apart from the others.
    pub strava_attempts: Arc<crate::auth::flow::Attempts>,
    /// Plugins' account connections in flight, apart from the others.
    pub plugin_attempts: Arc<crate::auth::flow::Attempts>,
    /// Each person's artifacts. Absent without a database.
    pub artifacts: Option<Arc<dyn niles_artifacts::ArtifactStore>>,
    /// What runs an artifact's job again, and says whether an assistant
    /// is there to. Answered by the binary.
    pub artifact_jobs: Option<Arc<dyn crate::artifacts::Jobs>>,
    /// Notifications on each person's devices. Absent without a
    /// database and its secrets key: the key that signs every push is
    /// not kept anywhere unsealed.
    pub push: Option<Arc<niles_webpush::WebPush>>,
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
            unifi: None,
            attempts: Arc::new(crate::auth::flow::Attempts::new()),
            manual_mode: None,
            scenes: None,
            groceries: None,
            automations: None,
            automation_runs: None,
            skills: None,
            memory: None,
            calendars: None,
            calendar_attempts: Arc::new(crate::auth::flow::Attempts::new()),
            chatgpt_logins: Arc::default(),
            chat: None,
            approvals: None,
            nemlig: None,
            plugins: None,
            tool_servers: None,
            strava: None,
            strava_attempts: Arc::new(crate::auth::flow::Attempts::new()),
            plugin_attempts: Arc::new(crate::auth::flow::Attempts::new()),
            artifacts: None,
            artifact_jobs: None,
            push: None,
            music: None,
            tado: None,
            voices: None,
            captures: None,
            secrets: None,
        }
    }

    /// The encrypted secret store, so Settings can fill it in.
    pub fn with_secrets(mut self, secrets: Option<Arc<dyn crate::secrets::SecretStore>>) -> Self {
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

    /// The UniFi console, for the one question only it can answer:
    /// which device is making this request.
    pub fn with_unifi(mut self, unifi: Option<Arc<niles_presence::UnifiSource>>) -> Self {
        self.unifi = unifi;
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

    pub fn with_calendars(mut self, calendars: Option<Arc<niles_calendar::Calendars>>) -> Self {
        self.calendars = calendars;
        self
    }

    pub fn with_automation_runs(mut self, runs: Option<Arc<niles_automations::RunLog>>) -> Self {
        self.automation_runs = runs;
        self
    }

    pub fn with_automations(
        mut self,
        automations: Option<Arc<niles_automations::AutomationStore>>,
    ) -> Self {
        self.automations = automations;
        self
    }

    pub fn with_skills(mut self, skills: Option<Arc<niles_skills::SkillStore>>) -> Self {
        self.skills = skills;
        self
    }

    pub fn with_memory(mut self, memory: Option<Arc<niles_memory::MemoryStore>>) -> Self {
        self.memory = memory;
        self
    }

    pub fn with_groceries(mut self, groceries: Option<Arc<niles_groceries::GroceryStore>>) -> Self {
        self.groceries = groceries;
        self
    }

    pub fn with_nemlig(mut self, nemlig: Option<Arc<niles_nemlig::NemligClient>>) -> Self {
        self.nemlig = nemlig;
        self
    }

    pub fn with_plugins(mut self, plugins: Option<Arc<dyn crate::plugins::Plugins>>) -> Self {
        self.plugins = plugins;
        self
    }

    pub fn with_tool_servers(
        mut self,
        servers: Option<Arc<dyn crate::tool_servers::ToolServers>>,
    ) -> Self {
        self.tool_servers = servers;
        self
    }

    pub fn with_strava(mut self, strava: Option<Arc<niles_training::Strava>>) -> Self {
        self.strava = strava;
        self
    }

    pub fn with_artifacts(
        mut self,
        store: Option<Arc<dyn niles_artifacts::ArtifactStore>>,
        jobs: Option<Arc<dyn crate::artifacts::Jobs>>,
    ) -> Self {
        self.artifacts = store;
        self.artifact_jobs = jobs;
        self
    }

    pub fn with_push(mut self, push: Option<Arc<niles_webpush::WebPush>>) -> Self {
        self.push = push;
        self
    }

    pub fn with_music(mut self, music: Option<Arc<dyn crate::music::Music>>) -> Self {
        self.music = music;
        self
    }

    pub fn with_chat(mut self, chat: Option<Arc<dyn crate::chat::Chat>>) -> Self {
        self.chat = chat;
        self
    }

    pub fn with_approvals(
        mut self,
        approvals: Option<Arc<dyn crate::approvals::Approvals>>,
    ) -> Self {
        self.approvals = approvals;
        self
    }

    pub fn with_manual_mode(mut self, tracker: Option<Arc<ManualModeTracker>>) -> Self {
        self.manual_mode = tracker;
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

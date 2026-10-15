//! The plugin runtime: one wasmtime engine, each plugin's instance on a
//! thread of its own, and the ways the rest of Niles reaches them.
//!
//! Every plugin has a [`Queue`] its thread works through, one call at a
//! time. Somebody waiting — a tool call, a command to a device, a page
//! being drawn — goes ahead of timers and events. The thread is never
//! one of the async runtime's workers, so a plugin that spins cannot
//! slow the voice loop; it is stopped by its deadline instead (see
//! [`crate::worker`]).

use crate::bindings::niles::integration::types as wit;
use crate::error::{Error, Result};
use crate::host::{HostState, class_to_wit, state_to_wit};
use crate::log::{Level, PluginLog};
use crate::manifest::{EventPermission, Manifest};
use crate::store::{KnownDevice, Package, PluginStore};
use crate::ui::{ActionOutcome, ActionValue, Context, Surface};
use async_trait::async_trait;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, PresenceState};
use niles_mqtt::DeviceCommander;
use niles_notifications::NotificationCenter;
use niles_tools::DynamicTools;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{
    Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak,
};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use wasmtime::component::Linker;
use wasmtime::{Engine, EngineWeak};

/// A plugin's settings, by plugin id.
pub type SettingsFn = Arc<dyn Fn(&str) -> toml::Table + Send + Sync>;
/// A plugin's secret, by plugin id and key.
pub type SecretsFn = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;
/// Told what happens on every plugin's MQTT connections.
pub type MqttWatch = Arc<dyn Fn(&crate::mqtt::Trace) + Send + Sync>;
/// The house's time zone, an IANA name, read each time a plugin asks:
/// it can change while plugins run.
pub type TimeZoneFn = Arc<dyn Fn() -> String + Send + Sync>;

/// The largest component Niles will run.
pub const MAX_COMPONENT: usize = 64 * 1024 * 1024;

/// Events waiting for a plugin, at most; past that the oldest go.
pub const EVENT_QUEUE: usize = 256;
/// Calls to `ui.invalidate` this close together become one message.
const COALESCE: Duration = Duration::from_millis(250);
/// One device's state reports this close together become one event.
pub const REPORT_INTERVAL: Duration = Duration::from_millis(100);
const LOG_WINDOW: Duration = Duration::from_secs(10);
const LOG_LINES: u32 = 100;
const LOG_LINE_BYTES: usize = 2048;

/// The spec's limits, in one place so tests can shorten them.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Most exports.
    pub call: Duration,
    pub init: Duration,
    pub render: Duration,
    pub stop: Duration,
    /// Waits before each restart after a trap or a timeout; the last
    /// one repeats.
    pub backoff: Vec<Duration>,
    /// This many restarts within an hour, and the next failure stops
    /// the plugin.
    pub restarts_per_hour: usize,
    /// How long after `init` returned an error it is tried again.
    pub init_retry: Duration,
    /// How often the engine's epoch advances: the resolution of every
    /// deadline above.
    pub tick: Duration,
    /// Let MQTT connect to this machine, where the tests' broker is.
    /// Only in the crate's own tests; Niles never has it.
    #[cfg(test)]
    pub(crate) mqtt_loopback: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            call: Duration::from_secs(10),
            init: Duration::from_secs(30),
            render: Duration::from_secs(3),
            stop: Duration::from_secs(2),
            backoff: vec![
                Duration::from_secs(5),
                Duration::from_secs(30),
                Duration::from_secs(2 * 60),
                Duration::from_secs(10 * 60),
                Duration::from_secs(30 * 60),
            ],
            restarts_per_hour: 5,
            init_retry: Duration::from_secs(10 * 60),
            tick: Duration::from_millis(100),
            #[cfg(test)]
            mqtt_loopback: false,
        }
    }
}

/// The parts of Niles a plugin's imports reach.
pub struct House {
    pub(crate) registry: Arc<DeviceRegistry>,
    pub(crate) bus: EventBus,
    pub(crate) tools: DynamicTools,
    pub(crate) store: Arc<dyn PluginStore>,
    pub(crate) notifications: OnceLock<Arc<NotificationCenter>>,
    pub(crate) settings: SettingsFn,
    pub(crate) secrets: SecretsFn,
    pub(crate) mqtt_watch: Option<MqttWatch>,
    pub(crate) time_zone: TimeZoneFn,
    pub(crate) handle: tokio::runtime::Handle,
}

impl House {
    /// Must be called on the Tokio runtime the plugins' I/O will use.
    pub fn new(registry: Arc<DeviceRegistry>, bus: EventBus, store: Arc<dyn PluginStore>) -> Self {
        Self {
            registry,
            bus,
            tools: DynamicTools::new(),
            store,
            notifications: OnceLock::new(),
            settings: Arc::new(|_| toml::Table::new()),
            secrets: Arc::new(|_, _| None),
            mqtt_watch: None,
            time_zone: Arc::new(|| "UTC".to_string()),
            handle: tokio::runtime::Handle::current(),
        }
    }

    /// Where plugin tools are offered. Attach the same handle to the
    /// tool registry the assistants use.
    pub fn with_tools(mut self, tools: DynamicTools) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_settings(mut self, settings: SettingsFn) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_secrets(mut self, secrets: SecretsFn) -> Self {
        self.secrets = secrets;
        self
    }

    /// What `config.time-zone` answers. UTC until this is set.
    pub fn with_time_zone(mut self, time_zone: TimeZoneFn) -> Self {
        self.time_zone = time_zone;
        self
    }

    /// Be told each connect, status, subscription, publish and message
    /// on the plugins' MQTT connections, as `niles plugin dev` prints them.
    pub fn with_mqtt_watch(mut self, watch: MqttWatch) -> Self {
        self.mqtt_watch = Some(watch);
        self
    }
}

/// How a plugin is doing, as its card says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    /// A required setting has no value; the plugin does not start.
    NotSetUp(String),
    /// It trapped, timed out, or its `init` said what is wrong. It is
    /// started again on its own.
    Failing(String),
    /// It kept failing and is not started again until somebody asks.
    Stopped(String),
}

impl Status {
    /// `starting`, `running`, `not_set_up`, `failing` or `stopped`.
    pub fn kind(&self) -> &'static str {
        match self {
            Status::Starting => "starting",
            Status::Running => "running",
            Status::NotSetUp(_) => "not_set_up",
            Status::Failing(_) => "failing",
            Status::Stopped(_) => "stopped",
        }
    }

    /// The card's status line.
    pub fn line(&self) -> String {
        match self {
            Status::Starting => "Starting".into(),
            Status::Running => "Working".into(),
            Status::NotSetUp(why) => format!("Not set up: {why}"),
            Status::Failing(why) => format!("Not working: {why}"),
            Status::Stopped(_) => "Stopped after repeated failures".into(),
        }
    }
}

/// One plugin, as listed.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: Status,
    /// The last thing that went wrong, if anything has.
    pub error: Option<String>,
    /// Events dropped because the plugin was not keeping up.
    pub dropped_events: u64,
    /// Traps, timeouts and a component that would not load, since this
    /// version started: what an update's rollback watches for.
    pub failures: u64,
}

/// Why a page or an action could not be answered.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceError {
    /// No such plugin, or it declares no such surface.
    NotFound(String),
    /// The plugin answered with an error of its own, or a tree Niles
    /// will not draw. Shown in place of the surface.
    Plugin(String),
    /// The plugin is not running; this is its status line.
    Unavailable(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceError::NotFound(s) | SurfaceError::Plugin(s) | SurfaceError::Unavailable(s) => {
                f.write_str(s)
            }
        }
    }
}

/// The runtime. Cheap to clone; every clone is the same runtime.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

struct Inner {
    engine: Engine,
    linker: Arc<Linker<HostState>>,
    house: Arc<House>,
    limits: Arc<Limits>,
    plugins: RwLock<BTreeMap<String, Arc<Plugin>>>,
    /// Each plugin's log, by id: it outlives the instance and the
    /// version that wrote it.
    logs: Mutex<HashMap<String, Arc<PluginLog>>>,
    images: crate::images::Images,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for plugin in self
            .plugins
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            plugin.queue.stop();
        }
    }
}

/// The engine every plugin runs in: component model, and epoch
/// interruption for the deadlines.
pub(crate) fn engine() -> Result<Engine> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|e| Error::Engine(e.to_string()))
}

/// Everything a plugin component may import: WASI 0.2 (empty, see
/// [`HostState::new`]), `wasi:http/types` without the outgoing handler,
/// and `niles:integration`.
pub(crate) fn linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    let fail = |e: wasmtime::Error| Error::Engine(e.to_string());
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(fail)?;
    let options = wasmtime_wasi_http::p2::bindings::LinkOptions::default();
    wasmtime_wasi_http::p2::bindings::http::types::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(
        &mut linker,
        &options.into(),
        <HostState as wasmtime_wasi_http::WasiHttpView>::http,
    )
    .map_err(fail)?;
    crate::bindings::add_to_linker(&mut linker).map_err(fail)?;
    Ok(linker)
}

/// Advance the engine's epoch every `tick` for as long as it exists.
fn spawn_ticker(engine: EngineWeak, tick: Duration) {
    std::thread::Builder::new()
        .name("plugin-epoch".into())
        .spawn(move || {
            while let Some(engine) = engine.upgrade() {
                engine.increment_epoch();
                drop(engine);
                std::thread::sleep(tick);
            }
        })
        .expect("a thread for the plugin deadlines");
}

impl Runtime {
    /// Start the runtime. Plugins are added with [`start`](Self::start)
    /// or [`load_installed`](Self::load_installed).
    pub fn new(house: House, limits: Limits) -> Result<Self> {
        let engine = engine()?;
        let linker = Arc::new(linker(&engine)?);
        spawn_ticker(engine.weak(), limits.tick);
        let inner = Arc::new(Inner {
            engine,
            linker,
            house: Arc::new(house),
            limits: Arc::new(limits),
            plugins: RwLock::new(BTreeMap::new()),
            logs: Mutex::default(),
            images: Default::default(),
        });
        spawn_forwarder(Arc::downgrade(&inner));
        Ok(Self { inner })
    }

    /// Let plugins speak, once the notification centre exists.
    pub fn connect_notifications(&self, center: Arc<NotificationCenter>) {
        let _ = self.inner.house.notifications.set(center);
    }

    /// Start every plugin the store holds as enabled. One that cannot be
    /// read or started is logged and shown as failing; none stops Niles.
    pub async fn load_installed(&self) -> usize {
        let installed = match self.inner.house.store.installed().await {
            Ok(installed) => installed,
            Err(e) => {
                tracing::error!("[plugins] could not read what is installed: {e}");
                return 0;
            }
        };
        self.remember_disabled().await;
        let mut started = 0;
        for plugin in installed {
            let manifest = match Manifest::parse(&plugin.manifest) {
                Ok(manifest) => manifest,
                Err(e) => {
                    tracing::error!("[plugins] an installed plugin's {e}; it is not started");
                    continue;
                }
            };
            match self.start(manifest, plugin.component).await {
                Ok(()) => started += 1,
                Err(e) => tracing::error!("[plugins] {e}"),
            }
        }
        started
    }

    /// A disabled plugin's devices stay in their rooms, unavailable, as
    /// they were when it was switched off, so the rules that name them
    /// still read.
    async fn remember_disabled(&self) {
        let Ok(records) = self.inner.house.store.records().await else {
            return;
        };
        for record in records.into_iter().filter(|r| !r.enabled) {
            if let Ok(manifest) = Manifest::parse(&record.manifest) {
                self.inner.remember_devices(&Plugin::new(manifest)).await;
            }
        }
    }

    /// Keep a plugin from a file, with no source, and start it. For
    /// tests and development: nothing here asks the household.
    pub async fn install(&self, manifest_text: &str, component: Vec<u8>) -> Result<()> {
        self.install_package(Package {
            manifest: manifest_text.to_string(),
            component,
            logo: None,
            source: None,
        })
        .await
    }

    /// Keep a plugin as installed and start it: what an approved
    /// install or update ends in. The manifest and component are checked;
    /// a plugin with the same id is replaced, and kept as the version a
    /// rollback goes back to.
    pub async fn install_package(&self, package: Package) -> Result<()> {
        let manifest = Manifest::parse_valid(&package.manifest)?;
        if package.component.len() > MAX_COMPONENT {
            return Err(Error::Component(format!(
                "is {} MiB; at most {} MiB",
                package.component.len() / (1024 * 1024),
                MAX_COMPONENT / (1024 * 1024)
            )));
        }
        let engine = self.inner.engine.clone();
        let linker = self.inner.linker.clone();
        let bytes = package.component.clone();
        let interface = manifest.interface.clone();
        tokio::task::spawn_blocking(move || {
            crate::worker::prepare(&engine, &linker, &bytes, &interface)
        })
        .await
        .map_err(|e| Error::Component(e.to_string()))?
        .map_err(Error::Component)?;
        self.inner
            .house
            .store
            .install(&manifest.id, &manifest.version, &package)
            .await
            .map_err(Error::Store)?;
        self.remove_running(&manifest.id).await;
        self.start(manifest, package.component).await
    }

    /// Stop a plugin and keep it stopped across restarts. Its settings,
    /// secrets, storage and devices stay; the devices are unavailable.
    pub async fn disable(&self, id: &str) -> Result<()> {
        if !self
            .inner
            .house
            .store
            .set_enabled(id, false)
            .await
            .map_err(Error::Store)?
        {
            return Err(Error::UnknownPlugin(id.to_string()));
        }
        self.remove_running(id).await;
        Ok(())
    }

    /// Start a disabled plugin, and keep it starting with Niles.
    pub async fn enable(&self, id: &str) -> Result<()> {
        let store = &self.inner.house.store;
        let installed = store
            .package(id)
            .await
            .map_err(Error::Store)?
            .ok_or_else(|| Error::UnknownPlugin(id.to_string()))?;
        store.set_enabled(id, true).await.map_err(Error::Store)?;
        if self.inner.plugins_read().contains_key(id) {
            return Ok(());
        }
        self.start(Manifest::parse(&installed.manifest)?, installed.component)
            .await
    }

    /// Put the version before the last update back, and start it. The
    /// version it replaces is not kept.
    pub async fn roll_back(&self, id: &str) -> Result<String> {
        let installed = self
            .inner
            .house
            .store
            .roll_back(id)
            .await
            .map_err(Error::Store)?
            .ok_or_else(|| Error::NoPrevious(id.to_string()))?;
        let manifest = Manifest::parse(&installed.manifest)?;
        let version = manifest.version.clone();
        self.remove_running(id).await;
        self.start(manifest, installed.component).await?;
        Ok(version)
    }

    /// Stop a plugin and forget it: its devices leave the house, with an
    /// event each, and its tools, storage and stored component go.
    /// Settings and secrets are kept elsewhere and are the caller's.
    pub async fn remove(&self, id: &str) -> Result<()> {
        let store = &self.inner.house.store;
        if store.package(id).await.map_err(Error::Store)?.is_none()
            && !self.inner.plugins_read().contains_key(id)
        {
            return Err(Error::UnknownPlugin(id.to_string()));
        }
        self.remove_running(id).await;
        lock(&self.inner.logs).remove(id);
        self.inner.images.forget(id);
        self.inner.house.tools.remove(id);
        let source = format!("x_{id}");
        for device in self.inner.house.registry.list_all() {
            if device.id.source() == source {
                self.inner.house.registry.remove(&device.id);
                self.inner
                    .house
                    .bus
                    .publish(Event::DeviceRemoved { id: device.id });
            }
        }
        store.remove(id).await.map_err(Error::Store)
    }

    pub(crate) fn store(&self) -> &Arc<dyn PluginStore> {
        &self.inner.house.store
    }

    pub(crate) fn registry(&self) -> &Arc<DeviceRegistry> {
        &self.inner.house.registry
    }

    pub(crate) fn dynamic_tools(&self) -> &DynamicTools {
        &self.inner.house.tools
    }

    /// Run `manifest`'s component. Its devices from before are put back
    /// in their rooms, unavailable until it reports them again.
    pub async fn start(&self, manifest: Manifest, component: Vec<u8>) -> Result<()> {
        let id = manifest.id.clone();
        if self.inner.plugins_read().contains_key(&id) {
            return Err(Error::AlreadyRunning(id));
        }
        let log = self.inner.log_of(&id);
        let plugin = Arc::new(Plugin::with_log(manifest, log));
        self.inner.remember_devices(&plugin).await;
        self.inner
            .plugins
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), plugin.clone());

        let problems = plugin.manifest.problems();
        if !problems.is_empty() {
            plugin.set_status(Status::Failing(format!(
                "the manifest is not valid: {}",
                problems.join("; ")
            )));
            return Ok(());
        }
        let engine = self.inner.engine.clone();
        let linker = self.inner.linker.clone();
        let house = self.inner.house.clone();
        let limits = self.inner.limits.clone();
        let worker_plugin = plugin.clone();
        let thread = std::thread::Builder::new()
            .name(format!("plugin-{id}"))
            .spawn(move || {
                crate::worker::run(worker_plugin, component, engine, linker, house, limits)
            })
            .map_err(|e| Error::Engine(format!("no thread for {id}: {e}")))?;
        *plugin.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);
        Ok(())
    }

    /// Stop a plugin's instance and start a fresh one, as saving its
    /// settings does. Also the way out of "stopped after repeated
    /// failures".
    pub fn restart(&self, id: &str) -> Result<()> {
        self.inner.plugin(id)?.queue.restart();
        Ok(())
    }

    /// Every plugin, as its card would list it.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.inner
            .plugins_read()
            .values()
            .map(|p| p.info())
            .collect()
    }

    pub fn info(&self, id: &str) -> Option<PluginInfo> {
        self.inner.plugin(id).ok().map(|p| p.info())
    }

    /// The plugin's last log lines, oldest first; empty for one that has
    /// written nothing since Niles started.
    pub fn log(&self, id: &str) -> Vec<crate::log::Line> {
        lock(&self.inner.logs)
            .get(id)
            .map(|log| log.lines())
            .unwrap_or_default()
    }

    /// A picture on one of a running plugin's pages, fetched by Niles
    /// (see [`crate::images`]).
    pub async fn image(
        &self,
        id: &str,
        src: &str,
    ) -> std::result::Result<crate::images::Image, crate::images::ImageError> {
        let manifest = self.manifest(id).ok_or_else(|| {
            crate::images::ImageError::NotFound(format!("No plugin {id} is running."))
        })?;
        self.inner.images.get(&manifest, src).await
    }

    /// The manifest of a running plugin.
    pub fn manifest(&self, id: &str) -> Option<Manifest> {
        self.inner.plugin(id).ok().map(|p| p.manifest.clone())
    }

    /// What the household entered for a plugin's settings.
    pub fn settings_of(&self, id: &str) -> toml::Table {
        (self.inner.house.settings)(id)
    }

    /// Whether a plugin's secret has a value.
    pub fn secret_is_set(&self, id: &str, key: &str) -> bool {
        (self.inner.house.secrets)(id, key).is_some()
    }

    /// The tools a plugin offers right now, as the model sees them.
    pub fn tools(&self, id: &str) -> Vec<niles_tools::ToolDescriptor> {
        self.inner.house.tools.descriptors_of(id)
    }

    /// Call one of a plugin's tools by its bare name.
    pub async fn call_tool(
        &self,
        id: &str,
        tool: &str,
        args_json: &str,
    ) -> std::result::Result<String, String> {
        let plugin = self.inner.plugin(id).map_err(|e| e.to_string())?;
        plugin.call_tool(tool, args_json).await
    }

    /// Draw a page or widget, or serve the last drawing while it is fresh.
    pub async fn render(
        &self,
        id: &str,
        surface: &str,
        context: Context,
    ) -> std::result::Result<Surface, SurfaceError> {
        let plugin = self.declared(id, surface)?;
        plugin.render(surface, context).await
    }

    /// Somebody used a control on a page or widget.
    pub async fn act(
        &self,
        id: &str,
        surface: &str,
        control: &str,
        value: ActionValue,
        context: Context,
    ) -> std::result::Result<ActionOutcome, SurfaceError> {
        let plugin = self.declared(id, surface)?;
        plugin.act(surface, control, value, context).await
    }

    fn declared(&self, id: &str, surface: &str) -> std::result::Result<Arc<Plugin>, SurfaceError> {
        let plugin = self
            .inner
            .plugin(id)
            .map_err(|e| SurfaceError::NotFound(e.to_string()))?;
        if !plugin.manifest.permissions.ui || !plugin.manifest.declares_surface(surface) {
            return Err(SurfaceError::NotFound(format!(
                "{} has no page or widget called {surface}",
                plugin.manifest.name
            )));
        }
        Ok(plugin)
    }

    /// Stop every plugin, giving each its `stop`, and wait for them.
    pub async fn shutdown(&self) {
        let plugins: Vec<Arc<Plugin>> = self.inner.plugins_read().values().cloned().collect();
        for plugin in &plugins {
            plugin.queue.stop();
        }
        for plugin in plugins {
            plugin.join().await;
        }
    }

    async fn remove_running(&self, id: &str) {
        let removed = self
            .inner
            .plugins
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Some(plugin) = removed {
            self.inner.house.tools.remove(id);
            plugin.queue.stop();
            plugin.join().await;
        }
    }
}

impl Inner {
    fn plugins_read(&self) -> RwLockReadGuard<'_, BTreeMap<String, Arc<Plugin>>> {
        self.plugins.read().unwrap_or_else(|e| e.into_inner())
    }

    fn log_of(&self, id: &str) -> Arc<PluginLog> {
        lock(&self.logs).entry(id.to_string()).or_default().clone()
    }

    fn plugin(&self, id: &str) -> Result<Arc<Plugin>> {
        self.plugins_read()
            .get(id)
            .cloned()
            .ok_or_else(|| Error::UnknownPlugin(id.to_string()))
    }

    fn by_source(&self, source: &str) -> Option<Arc<Plugin>> {
        let id = source.strip_prefix("x_")?;
        self.plugins_read().get(id).cloned()
    }

    /// Put the devices Niles holds for `plugin` back in their rooms,
    /// unavailable, so rules that name them stay valid.
    async fn remember_devices(&self, plugin: &Plugin) {
        let known = match self.house.store.devices(&plugin.id).await {
            Ok(known) => known,
            Err(e) => {
                tracing::warn!(plugin = %plugin.id, "could not read its devices: {e}");
                return;
            }
        };
        let mut devices = plugin.devices_mut();
        for device in known {
            let Some((room, name)) = device.id.split_once('/') else {
                continue;
            };
            let Ok(id) = DeviceId::parse(&format!("{}:{room}/{name}", plugin.source)) else {
                continue;
            };
            if self.house.registry.get(&id).is_none() {
                let mut remembered = device.device(id.clone(), DeviceState::default());
                remembered.available = false;
                self.house.registry.upsert(remembered.clone());
                self.house
                    .bus
                    .publish(Event::DeviceAdded { device: remembered });
            }
            devices.insert(id, device);
        }
    }

    /// Hand `event` to every plugin that subscribed to its kind, except
    /// the one whose device it is.
    fn forward(&self, event: &Event) {
        let (kind, device) = match event {
            Event::DeviceStateChanged { id, .. } => (EventPermission::DeviceState, Some(id)),
            Event::DeviceAction { id, .. } => (EventPermission::DeviceAction, Some(id)),
            Event::PresenceChanged { .. } => (EventPermission::Presence, None),
            _ => return,
        };
        let mut converted = None;
        for plugin in self.plugins_read().values() {
            if let Some(id) = device
                && id.source() == plugin.source
            {
                continue;
            }
            if !plugin.wants(kind, device) {
                continue;
            }
            let event = converted.get_or_insert_with(|| self.to_wit(event));
            if let Some(event) = event {
                plugin.queue.push_event(event.clone());
            }
        }
    }

    fn to_wit(&self, event: &Event) -> Option<wit::Event> {
        Some(match event {
            Event::DeviceStateChanged { id, state } => {
                let class = self
                    .house
                    .registry
                    .get(id)
                    .map(|d| class_to_wit(d.class))
                    .unwrap_or(wit::DeviceClass::Sensor);
                wit::Event::DeviceState(wit::DeviceStateChanged {
                    id: id.to_string(),
                    class,
                    state: state_to_wit(state),
                })
            }
            Event::DeviceAction { id, action } => {
                wit::Event::DeviceAction(wit::DeviceActionFired {
                    id: id.to_string(),
                    action: action.clone(),
                })
            }
            Event::PresenceChanged { state } => wit::Event::Presence(match state {
                PresenceState::Home => wit::HomeState::Home,
                PresenceState::Away => wit::HomeState::Away,
                _ => wit::HomeState::Unknown,
            }),
            _ => return None,
        })
    }
}

fn spawn_forwarder(inner: Weak<Inner>) {
    let Some(strong) = inner.upgrade() else {
        return;
    };
    let mut events = strong.house.bus.subscribe();
    let handle = strong.house.handle.clone();
    drop(strong);
    handle.spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => match inner.upgrade() {
                    Some(inner) => inner.forward(&event),
                    None => break,
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[plugins] missed {n} events from the house");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[async_trait]
impl DeviceCommander for Runtime {
    /// Every `x_` source is a plugin's, whether or not that plugin is
    /// running: a command to one never falls through to MQTT.
    fn handles(&self, id: &DeviceId) -> bool {
        id.source().starts_with("x_")
    }

    async fn command(
        &self,
        id: &DeviceId,
        desired: &DeviceState,
    ) -> std::result::Result<(), String> {
        let spoken = format!("{} {}", id.room(), id.name()).replace('_', " ");
        let plugin = self
            .inner
            .by_source(id.source())
            .ok_or_else(|| format!("The {spoken}'s integration is not installed."))?;
        let known =
            plugin.devices().get(id).cloned().ok_or_else(|| {
                format!("{} has no device called {spoken}.", plugin.manifest.name)
            })?;
        if !known.controllable {
            return Err(format!("The {spoken} cannot be switched."));
        }
        // A reading is measured, never set.
        let mut desired = desired.clone();
        desired.readings.clear();
        let (reply, answer) = oneshot::channel();
        plugin.queue.push(Job::SetDevice {
            id: known.id,
            desired: state_to_wit(&desired),
            reply,
        });
        answer.await.unwrap_or_else(|_| Err(plugin.not_working()))
    }
}

/// One plugin, shared by its thread, its tools and the runtime.
pub(crate) struct Plugin {
    pub id: String,
    pub source: String,
    pub manifest: Manifest,
    pub queue: Queue,
    record: Mutex<Record>,
    devices: RwLock<HashMap<DeviceId, KnownDevice>>,
    subscriptions: Mutex<HashMap<EventPermission, Vec<String>>>,
    renders: Mutex<HashMap<(String, Context), (Instant, Surface)>>,
    pending_invalidations: Mutex<HashSet<String>>,
    reports: Mutex<HashMap<DeviceId, Reported>>,
    notified: Mutex<VecDeque<Instant>>,
    log_window: Mutex<LogWindow>,
    log: Arc<PluginLog>,
    failures: std::sync::atomic::AtomicU64,
    /// The last MQTT connection id handed out. Ids are never reused, so
    /// a message for a connection that is gone cannot reach a new one.
    connections: std::sync::atomic::AtomicU32,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

struct Record {
    status: Status,
    error: Option<String>,
}

/// When a device's state change was last published, and what has been
/// reported since and not yet published.
struct Reported {
    at: Instant,
    waiting: Option<DeviceState>,
}

struct LogWindow {
    start: Instant,
    lines: u32,
    dropped: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Plugin {
    pub(crate) fn new(manifest: Manifest) -> Self {
        Self::with_log(manifest, Arc::default())
    }

    fn with_log(manifest: Manifest, log: Arc<PluginLog>) -> Self {
        Self {
            id: manifest.id.clone(),
            source: manifest.source(),
            manifest,
            queue: Queue::default(),
            record: Mutex::new(Record {
                status: Status::Starting,
                error: None,
            }),
            devices: RwLock::default(),
            subscriptions: Mutex::default(),
            renders: Mutex::default(),
            pending_invalidations: Mutex::default(),
            reports: Mutex::default(),
            notified: Mutex::default(),
            log_window: Mutex::new(LogWindow {
                start: Instant::now(),
                lines: 0,
                dropped: 0,
            }),
            log,
            failures: Default::default(),
            connections: Default::default(),
            thread: Mutex::default(),
        }
    }

    pub fn info(&self) -> PluginInfo {
        let record = lock(&self.record);
        PluginInfo {
            id: self.id.clone(),
            name: self.manifest.name.clone(),
            version: self.manifest.version.clone(),
            status: record.status.clone(),
            error: record.error.clone(),
            dropped_events: self.queue.dropped(),
            failures: self.failures.load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// The instance trapped, timed out, or could not be made at all.
    pub fn count_failure(&self) {
        self.failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn status(&self) -> Status {
        lock(&self.record).status.clone()
    }

    pub fn set_status(&self, status: Status) {
        let mut record = lock(&self.record);
        if record.status != status {
            let level = match &status {
                Status::Starting | Status::Running => Level::Info,
                Status::NotSetUp(_) => Level::Warn,
                Status::Failing(_) | Status::Stopped(_) => Level::Error,
            };
            let line = match &status {
                Status::Stopped(why) => format!("Stopped after repeated failures: {why}"),
                other => other.line(),
            };
            self.log.push(level, &line, true);
        }
        match &status {
            Status::Failing(why) | Status::Stopped(why) | Status::NotSetUp(why) => {
                record.error = Some(why.clone());
            }
            Status::Starting | Status::Running => {}
        }
        record.status = status;
    }

    /// A line in the plugin's log from Niles, about the plugin.
    pub fn note(&self, level: Level, line: &str) {
        self.log.push(level, line, true);
        let id = &self.id;
        match level {
            Level::Debug => tracing::debug!(plugin = %id, "{line}"),
            Level::Info => tracing::info!(plugin = %id, "{line}"),
            Level::Warn => tracing::warn!(plugin = %id, "{line}"),
            Level::Error => tracing::error!(plugin = %id, "{line}"),
        }
    }

    /// An id for a new MQTT connection.
    pub fn next_connection(&self) -> u32 {
        self.connections
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// What a caller is told while the plugin is down.
    pub fn not_working(&self) -> String {
        format!("{} is not working right now.", self.manifest.name)
    }

    pub fn devices(&self) -> RwLockReadGuard<'_, HashMap<DeviceId, KnownDevice>> {
        self.devices.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn devices_mut(&self) -> RwLockWriteGuard<'_, HashMap<DeviceId, KnownDevice>> {
        self.devices.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn subscribe(&self, kind: EventPermission, patterns: Vec<String>) {
        lock(&self.subscriptions).insert(kind, patterns);
    }

    pub fn unsubscribe(&self, kind: EventPermission) {
        lock(&self.subscriptions).remove(&kind);
    }

    /// Whether an event of `kind`, about `device` if it is about one,
    /// is one this plugin asked for.
    fn wants(&self, kind: EventPermission, device: Option<&DeviceId>) -> bool {
        let subscriptions = lock(&self.subscriptions);
        let Some(patterns) = subscriptions.get(&kind) else {
            return false;
        };
        match device {
            Some(id) if !patterns.is_empty() => patterns.iter().any(|p| {
                let (room, name) = p.split_once('/').unwrap_or((p, "*"));
                (room == "*" || room == id.room().as_str())
                    && (name == "*" || name == id.name().as_str())
            }),
            _ => true,
        }
    }

    /// Forget what the instance set up: it is gone.
    pub fn forget_instance(&self, house: &House) {
        lock(&self.subscriptions).clear();
        lock(&self.renders).clear();
        self.queue.clear_events();
        let ids: Vec<DeviceId> = self.devices().keys().cloned().collect();
        for id in ids {
            if house.registry.set_available(&id, false) {
                let state = house.registry.get(&id).map(|d| d.state).unwrap_or_default();
                house.bus.publish(Event::DeviceStateChanged { id, state });
            }
        }
    }

    /// One more notification, if the hour has room for it.
    pub fn may_notify(&self, per_hour: usize) -> bool {
        let mut sent = lock(&self.notified);
        let now = Instant::now();
        while sent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600))
        {
            sent.pop_front();
        }
        if sent.len() >= per_hour {
            return false;
        }
        sent.push_back(now);
        true
    }

    /// A line in the plugin's log, within its rate limit.
    pub fn log(&self, level: crate::bindings::niles::integration::log::Level, message: &str) {
        use crate::bindings::niles::integration::log::Level;
        {
            let mut window = lock(&self.log_window);
            if window.start.elapsed() >= LOG_WINDOW {
                if window.dropped > 0 {
                    let note = format!("{} log lines over the limit were not kept", window.dropped);
                    tracing::warn!(plugin = %self.id, "{note}");
                    self.log.push(crate::log::Level::Warn, &note, true);
                }
                *window = LogWindow {
                    start: Instant::now(),
                    lines: 0,
                    dropped: 0,
                };
            }
            if window.lines >= LOG_LINES {
                window.dropped += 1;
                return;
            }
            window.lines += 1;
        }
        let mut cut = message.len().min(LOG_LINE_BYTES);
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        let message = &message[..cut];
        self.log.push(level.into(), message, false);
        let id = &self.id;
        match level {
            Level::Debug => tracing::debug!(plugin = %id, "{message}"),
            Level::Info => tracing::info!(plugin = %id, "{message}"),
            Level::Warn => tracing::warn!(plugin = %id, "{message}"),
            Level::Error => tracing::error!(plugin = %id, "{message}"),
        }
    }

    /// Ask open apps to draw `surface` again, a moment from now so a
    /// burst of calls is one message.
    /// Publish a state change a plugin reported, at most once per
    /// [`REPORT_INTERVAL`] per device: reports in between are merged and
    /// published together when it has passed. The registry already has
    /// each one; this only spares the bus, and everything listening to
    /// it, a plugin that reports in a tight loop.
    pub fn publish_state(self: &Arc<Self>, id: DeviceId, partial: DeviceState, house: &House) {
        let now = Instant::now();
        let mut reports = lock(&self.reports);
        let reported = reports.entry(id.clone()).or_insert(Reported {
            at: now.checked_sub(REPORT_INTERVAL).unwrap_or(now),
            waiting: None,
        });
        if let Some(waiting) = &mut reported.waiting {
            waiting.merge(partial);
            return;
        }
        let since = now.duration_since(reported.at);
        if since >= REPORT_INTERVAL {
            reported.at = now;
            drop(reports);
            house
                .bus
                .publish(Event::DeviceStateChanged { id, state: partial });
            return;
        }
        reported.waiting = Some(partial);
        drop(reports);
        let plugin = self.clone();
        let bus = house.bus.clone();
        house.handle.spawn(async move {
            tokio::time::sleep(REPORT_INTERVAL - since).await;
            let state = {
                let mut reports = lock(&plugin.reports);
                let Some(reported) = reports.get_mut(&id) else {
                    return;
                };
                reported.at = Instant::now();
                reported.waiting.take()
            };
            if let Some(state) = state {
                bus.publish(Event::DeviceStateChanged { id, state });
            }
        });
    }

    pub fn invalidate(self: &Arc<Self>, surface: &str, house: &House) {
        lock(&self.renders).retain(|(s, _), _| s != surface);
        if !lock(&self.pending_invalidations).insert(surface.to_string()) {
            return;
        }
        let plugin = self.clone();
        let bus = house.bus.clone();
        let surface = surface.to_string();
        house.handle.spawn(async move {
            tokio::time::sleep(COALESCE).await;
            lock(&plugin.pending_invalidations).remove(&surface);
            bus.publish(Event::UiInvalidated {
                plugin: plugin.id.clone(),
                surface,
            });
        });
    }

    pub async fn call_tool(
        &self,
        tool: &str,
        args_json: &str,
    ) -> std::result::Result<String, String> {
        let (reply, answer) = oneshot::channel();
        self.queue.push(Job::CallTool {
            name: tool.to_string(),
            args: args_json.to_string(),
            reply,
        });
        answer.await.unwrap_or_else(|_| Err(self.not_working()))
    }

    async fn render(
        &self,
        surface: &str,
        context: Context,
    ) -> std::result::Result<Surface, SurfaceError> {
        let key = (surface.to_string(), context.clone());
        let fresh_for = self
            .manifest
            .refresh_every(surface)
            .map(|s| Duration::from_secs(u64::from(s)));
        if let Some((at, cached)) = lock(&self.renders).get(&key)
            && fresh_for.is_none_or(|limit| at.elapsed() < limit)
        {
            return Ok(cached.clone());
        }
        let (reply, answer) = oneshot::channel();
        self.queue.push(Job::Render {
            surface: surface.to_string(),
            context,
            reply,
        });
        let drawn = answer
            .await
            .unwrap_or_else(|_| Err(SurfaceError::Unavailable(self.status().line())))?;
        lock(&self.renders).insert(key, (Instant::now(), drawn.clone()));
        Ok(drawn)
    }

    async fn act(
        &self,
        surface: &str,
        control: &str,
        value: ActionValue,
        context: Context,
    ) -> std::result::Result<ActionOutcome, SurfaceError> {
        let (reply, answer) = oneshot::channel();
        self.queue.push(Job::Action {
            surface: surface.to_string(),
            control: control.to_string(),
            value,
            context,
            reply,
        });
        let outcome = answer
            .await
            .unwrap_or_else(|_| Err(SurfaceError::Unavailable(self.status().line())))?;
        if outcome.rerender {
            lock(&self.renders).retain(|(s, _), _| s != surface);
        }
        Ok(outcome)
    }

    async fn join(&self) {
        let thread = lock(&self.thread).take();
        if let Some(thread) = thread {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
    }
}

/// Something a plugin's thread has to do for somebody waiting.
pub(crate) enum Job {
    CallTool {
        name: String,
        args: String,
        reply: oneshot::Sender<std::result::Result<String, String>>,
    },
    SetDevice {
        /// `room/name`.
        id: String,
        desired: wit::DeviceState,
        reply: oneshot::Sender<std::result::Result<(), String>>,
    },
    Render {
        surface: String,
        context: Context,
        reply: oneshot::Sender<std::result::Result<Surface, SurfaceError>>,
    },
    Action {
        surface: String,
        control: String,
        value: ActionValue,
        context: Context,
        reply: oneshot::Sender<std::result::Result<ActionOutcome, SurfaceError>>,
    },
}

/// What a plugin's thread does next.
pub(crate) enum Next {
    Job(Job),
    Restart,
    Stop,
    /// A timer is due.
    Timers,
    /// It is time to start an instance.
    Start,
    Event(wit::Event),
    Mqtt(crate::mqtt::Delivery),
}

/// A plugin's work, in the order it is done: jobs somebody is waiting
/// for, then due timers, then events and MQTT deliveries, taking turns.
#[derive(Default)]
pub(crate) struct Queue {
    pending: Mutex<Pending>,
    ready: Condvar,
}

#[derive(Default)]
struct Pending {
    jobs: VecDeque<Job>,
    events: VecDeque<wit::Event>,
    dropped: u64,
    mqtt: crate::mqtt::Inbox,
    /// Whether an event goes next when both are waiting.
    events_turn: bool,
    restart: bool,
    stop: bool,
}

impl Queue {
    fn change(&self, f: impl FnOnce(&mut Pending)) {
        f(&mut lock(&self.pending));
        self.ready.notify_all();
    }

    /// Queue `job`. Once the queue is stopped nothing will answer it, so
    /// it is dropped, and with it the reply: whoever waits is told the
    /// plugin is not working rather than waiting for ever.
    pub fn push(&self, job: Job) {
        self.change(|p| {
            if !p.stop {
                p.jobs.push_back(job);
            }
        });
    }

    pub fn push_event(&self, event: wit::Event) {
        self.change(|p| {
            if p.events.len() >= EVENT_QUEUE {
                p.events.pop_front();
                p.dropped += 1;
            }
            p.events.push_back(event);
        });
    }

    pub fn push_mqtt(&self, delivery: crate::mqtt::Delivery) {
        self.change(|p| p.mqtt.push(delivery));
    }

    /// MQTT messages dropped since this was last asked.
    pub fn take_mqtt_dropped(&self) -> u64 {
        lock(&self.pending).mqtt.take_dropped()
    }

    pub fn restart(&self) {
        self.change(|p| p.restart = true);
    }

    pub fn stop(&self) {
        self.change(|p| {
            p.stop = true;
            p.jobs.clear();
        });
    }

    /// Forget the events and MQTT deliveries waiting: their instance is gone.
    pub fn clear_events(&self) {
        let mut pending = lock(&self.pending);
        pending.events.clear();
        pending.mqtt.clear();
    }

    pub fn dropped(&self) -> u64 {
        lock(&self.pending).dropped
    }

    /// Wait for the next thing to do. `timers` is when the earliest
    /// timer is due and `start` when an instance should start, if either.
    pub fn next(&self, timers: Option<Instant>, start: Option<Instant>) -> Next {
        let mut pending = lock(&self.pending);
        loop {
            if pending.stop {
                return Next::Stop;
            }
            if let Some(job) = pending.jobs.pop_front() {
                return Next::Job(job);
            }
            if std::mem::take(&mut pending.restart) {
                return Next::Restart;
            }
            let now = Instant::now();
            if timers.is_some_and(|t| t <= now) {
                return Next::Timers;
            }
            if start.is_some_and(|t| t <= now) {
                return Next::Start;
            }
            let mqtt_waiting = !pending.mqtt.is_empty();
            if !pending.events.is_empty() && (pending.events_turn || !mqtt_waiting) {
                pending.events_turn = false;
                if let Some(event) = pending.events.pop_front() {
                    return Next::Event(event);
                }
            }
            if let Some(delivery) = pending.mqtt.pop() {
                pending.events_turn = true;
                return Next::Mqtt(delivery);
            }
            pending = match [timers, start].into_iter().flatten().min() {
                Some(at) => {
                    self.ready
                        .wait_timeout(pending, at.saturating_duration_since(now))
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                }
                None => self.ready.wait(pending).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_read_like_the_card() {
        assert_eq!(Status::Running.line(), "Working");
        assert_eq!(
            Status::Failing("timed out in on-timer after 10 s".into()).line(),
            "Not working: timed out in on-timer after 10 s"
        );
        assert_eq!(
            Status::Stopped("x".into()).line(),
            "Stopped after repeated failures"
        );
        assert_eq!(Status::NotSetUp("x".into()).kind(), "not_set_up");
    }

    fn event() -> wit::Event {
        wit::Event::Presence(wit::HomeState::Home)
    }

    #[test]
    fn somebody_waiting_goes_ahead_of_events() {
        let queue = Queue::default();
        queue.push_event(event());
        let (reply, _answer) = oneshot::channel();
        queue.push(Job::CallTool {
            name: "x".into(),
            args: "{}".into(),
            reply,
        });
        assert!(matches!(queue.next(None, None), Next::Job(_)));
        assert!(matches!(queue.next(None, None), Next::Event(_)));
    }

    #[test]
    fn a_due_timer_goes_ahead_of_events() {
        let queue = Queue::default();
        queue.push_event(event());
        assert!(matches!(
            queue.next(Some(Instant::now()), None),
            Next::Timers
        ));
    }

    #[test]
    fn events_and_mqtt_take_turns() {
        let queue = Queue::default();
        let message = |n: u8| crate::mqtt::Delivery::Message {
            conn: 1,
            topic: "t".into(),
            payload: vec![n],
            latest_only: false,
        };
        queue.push_event(event());
        queue.push_event(event());
        queue.push_mqtt(message(1));
        queue.push_mqtt(message(2));
        let kinds: Vec<&str> = (0..4)
            .map(|_| match queue.next(None, None) {
                Next::Event(_) => "event",
                Next::Mqtt(_) => "mqtt",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["mqtt", "event", "mqtt", "event"]);
    }

    #[test]
    fn a_gone_instances_mqtt_deliveries_go_with_it() {
        let queue = Queue::default();
        queue.push_mqtt(crate::mqtt::Delivery::Status {
            conn: 1,
            status: crate::bindings::mqtt::ConnectionStatus::Connected,
        });
        queue.clear_events();
        let (reply, _answer) = oneshot::channel();
        queue.push(Job::CallTool {
            name: "x".into(),
            args: "{}".into(),
            reply,
        });
        assert!(matches!(queue.next(None, None), Next::Job(_)));
        assert!(lock(&queue.pending).mqtt.is_empty());
    }

    #[test]
    fn the_event_queue_drops_the_oldest_and_counts_them() {
        let queue = Queue::default();
        for _ in 0..EVENT_QUEUE + 3 {
            queue.push_event(event());
        }
        assert_eq!(queue.dropped(), 3);
    }

    #[tokio::test]
    async fn a_job_for_a_stopped_plugin_is_answered_not_kept() {
        let queue = Queue::default();
        let (reply, answer) = oneshot::channel();
        queue.push(Job::CallTool {
            name: "x".into(),
            args: "{}".into(),
            reply,
        });
        queue.stop();
        assert!(answer.await.is_err(), "dropped, so the caller hears");
        let (reply, answer) = oneshot::channel();
        queue.push(Job::CallTool {
            name: "x".into(),
            args: "{}".into(),
            reply,
        });
        assert!(answer.await.is_err());
    }

    #[test]
    fn a_stop_ends_the_wait() {
        let queue = Arc::new(Queue::default());
        let waiting = queue.clone();
        let thread = std::thread::spawn(move || matches!(waiting.next(None, None), Next::Stop));
        std::thread::sleep(Duration::from_millis(20));
        queue.stop();
        assert!(thread.join().unwrap());
    }

    #[test]
    fn six_notifications_an_hour() {
        let plugin = Plugin::new(
            Manifest::parse(include_str!(
                "../tests/fixtures/hello/niles-integration.toml"
            ))
            .unwrap(),
        );
        for _ in 0..6 {
            assert!(plugin.may_notify(6));
        }
        assert!(!plugin.may_notify(6));
    }

    #[test]
    fn device_patterns_narrow_by_room_and_name() {
        let plugin = Plugin::new(
            Manifest::parse(include_str!(
                "../tests/fixtures/hello/niles-integration.toml"
            ))
            .unwrap(),
        );
        let door = DeviceId::parse("z2m:office/door").unwrap();
        let lamp = DeviceId::parse("z2m:office/lamp").unwrap();
        assert!(!plugin.wants(EventPermission::DeviceState, Some(&door)));
        plugin.subscribe(EventPermission::DeviceState, vec!["*/door".into()]);
        assert!(plugin.wants(EventPermission::DeviceState, Some(&door)));
        assert!(!plugin.wants(EventPermission::DeviceState, Some(&lamp)));
        plugin.subscribe(EventPermission::DeviceState, Vec::new());
        assert!(plugin.wants(EventPermission::DeviceState, Some(&lamp)));
    }
}

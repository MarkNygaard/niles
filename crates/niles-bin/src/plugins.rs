//! Third-party plugins in the binary: `niles plugin check`, `niles
//! plugin dev`, and what `serve` needs to run the installed ones.

use anyhow::{Context as _, bail};
use clap::{Args, Subcommand};
use niles_api::plugins::{PluginFailure, PluginSummary};
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_mqtt::DeviceCommander;
use niles_plugins::{
    ActionValue, Context, House, Limits, Manifest, MemoryStore, PluginStore, Runtime, Status,
    SurfaceError,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Subcommand)]
pub(crate) enum PluginAction {
    /// Check a plugin the way an install would: the manifest, the logo,
    /// the component's world and version, and its tools.
    Check(CheckArgs),
    /// Run one plugin against a stand-in house, with its log on the
    /// terminal: devices and their changes, tools, timers and pages.
    Dev(DevArgs),
}

#[derive(Args)]
pub(crate) struct CheckArgs {
    /// The plugin's directory (with niles-integration.toml), its .wasm,
    /// or its GitHub repo (`jane/niles-spotprice`, or the repo's or a
    /// release's address), whose latest release is checked.
    path: PathBuf,
}

#[derive(Args)]
pub(crate) struct DevArgs {
    /// The plugin's directory (with niles-integration.toml), or its .wasm.
    /// From a directory, the newest target/wasm32-wasip2/release/*.wasm
    /// is run, or integration.wasm if there is one.
    path: PathBuf,
    /// A setting for this run, as key=value. Repeatable. Over
    /// dev-settings.toml, which is read from the plugin's directory.
    #[arg(long = "settings", value_name = "KEY=VALUE")]
    settings: Vec<String>,
    /// Start it, do what --call and --render ask, and stop, rather than
    /// staying up for its timers and the prompt.
    #[arg(long)]
    dry_run: bool,
    /// After init, call a tool by its bare name with JSON arguments.
    #[arg(long, num_args = 2, value_names = ["TOOL", "JSON"])]
    call: Option<Vec<String>>,
    /// After init, draw a page or widget as text.
    #[arg(long, value_name = "SURFACE")]
    render: Option<String>,
}

pub(crate) async fn run(action: PluginAction) -> anyhow::Result<()> {
    match action {
        PluginAction::Check(args) => check(args).await,
        PluginAction::Dev(args) => dev(args).await,
    }
}

/// A plugin on disk.
struct Found {
    dir: PathBuf,
    manifest: String,
    component: Option<(PathBuf, Vec<u8>)>,
}

fn locate(path: &Path) -> anyhow::Result<Found> {
    let (dir, wasm) = if path.is_dir() {
        let wasm = Some(path.join("integration.wasm"))
            .filter(|p| p.is_file())
            .or_else(|| newest_wasm(&path.join("target").join("wasm32-wasip2").join("release")));
        (path.to_path_buf(), wasm)
    } else if path.extension().is_some_and(|e| e == "wasm") {
        let dir = path
            .ancestors()
            .skip(1)
            .take(5)
            .find(|d| d.join("niles-integration.toml").is_file())
            .with_context(|| {
                format!(
                    "no niles-integration.toml beside or above {}",
                    path.display()
                )
            })?;
        (dir.to_path_buf(), Some(path.to_path_buf()))
    } else {
        bail!(
            "{} is neither a plugin's directory nor a .wasm",
            path.display()
        );
    };
    let manifest_path = dir.join("niles-integration.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let component = match wasm {
        Some(wasm) => {
            let bytes =
                std::fs::read(&wasm).with_context(|| format!("reading {}", wasm.display()))?;
            Some((wasm, bytes))
        }
        None => None,
    };
    Ok(Found {
        dir,
        manifest,
        component,
    })
}

fn newest_wasm(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

async fn check(args: CheckArgs) -> anyhow::Result<()> {
    if !args.path.exists()
        && let Some(text) = args.path.to_str()
        && let Ok((repo, tag)) = niles_plugins::github::parse_repo(text)
    {
        return check_release(repo, tag).await;
    }
    let found = locate(&args.path)?;
    let logo = Manifest::parse(&found.manifest)
        .ok()
        .and_then(|m| m.logo)
        .and_then(|logo| std::fs::read(found.dir.join(logo)).ok());
    if let Some((path, _)) = &found.component {
        println!("Checking {}", path.display());
    } else {
        println!("Checking {}", found.dir.display());
    }
    let report =
        niles_plugins::check::check(&found.manifest, found.component.map(|(_, b)| b), logo).await;
    print_report(&report)
}

/// Check a release on GitHub, downloading what an install would.
async fn check_release(
    repo: niles_plugins::github::Repo,
    tag: Option<String>,
) -> anyhow::Result<()> {
    use niles_plugins::github::{GitHub, MAX_MANIFEST};
    let github = GitHub::default();
    let release = github
        .release(&repo, tag.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("Checking {} {}", repo.slug(), release.tag_name);
    let fetch = |name: String, cap: usize| {
        let asset = release.asset(&name).cloned();
        let github = github.clone();
        async move {
            match asset {
                None => Ok(None),
                Some(asset) => github
                    .download(&asset, cap)
                    .await
                    .map(Some)
                    .map_err(|e| anyhow::anyhow!("{name}: {e}")),
            }
        }
    };
    let manifest = fetch("niles-integration.toml".into(), MAX_MANIFEST)
        .await?
        .context("the release has no niles-integration.toml")?;
    let manifest = String::from_utf8(manifest).context("niles-integration.toml is not UTF-8")?;
    let logo_name = Manifest::parse(&manifest)
        .ok()
        .and_then(|m| m.logo)
        .map(|logo| logo.rsplit('/').next().unwrap_or(&logo).to_string());
    let logo = match logo_name {
        Some(name) => fetch(name, niles_plugins::check::MAX_LOGO).await?,
        None => None,
    };
    let component = fetch(
        "integration.wasm".into(),
        niles_plugins::runtime::MAX_COMPONENT,
    )
    .await?;
    let mut report = niles_plugins::check::check(&manifest, component, logo).await;
    if let Some(m) = &report.manifest
        && release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name)
            != m.version
    {
        report.problems.push(format!(
            "release: tagged {}, and the manifest says {}",
            release.tag_name, m.version
        ));
    }
    print_report(&report)
}

fn print_report(report: &niles_plugins::check::Report) -> anyhow::Result<()> {
    for passed in &report.passed {
        println!("  ok       {passed}");
    }
    if !report.imports.is_empty() {
        println!("  imports  {}", report.imports.join(", "));
    }
    for tool in &report.tools {
        println!(
            "  tool     {}: {}",
            tool.name,
            first_line(&tool.description)
        );
    }
    for problem in &report.problems {
        println!("  PROBLEM  {problem}");
    }
    if report.ok() {
        println!("It would install.");
        Ok(())
    } else {
        bail!(
            "{} problem{} to fix before it installs",
            report.problems.len(),
            if report.problems.len() == 1 { "" } else { "s" }
        )
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// The settings for a dev run: dev-settings.toml, then `--settings`,
/// each typed by the manifest. Secrets come back separately.
fn dev_settings(
    dir: &Path,
    manifest: &Manifest,
    overrides: &[String],
) -> anyhow::Result<(toml::Table, HashMap<String, String>)> {
    let mut values = toml::Table::new();
    let mut secrets = HashMap::new();
    let file = dir.join("dev-settings.toml");
    if file.is_file() {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        for (key, value) in table {
            match manifest.setting(&key) {
                Some(s) if s.kind == niles_plugins::manifest::SettingKind::Secret => {
                    let text = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or(value.to_string());
                    secrets.insert(key, text);
                }
                Some(_) => {
                    values.insert(key, value);
                }
                None => println!("dev-settings.toml: {key} is not a setting the manifest declares"),
            }
        }
    }
    for pair in overrides {
        let (key, text) = pair
            .split_once('=')
            .with_context(|| format!("--settings {pair:?}: write it as key=value"))?;
        let setting = manifest
            .setting(key)
            .with_context(|| format!("--settings: the manifest declares no setting {key:?}"))?;
        if setting.kind == niles_plugins::manifest::SettingKind::Secret {
            secrets.insert(key.to_string(), text.to_string());
        } else {
            let value = setting
                .parse_text(text)
                .map_err(|why| anyhow::anyhow!("--settings {key}: {why}"))?;
            values.insert(key.to_string(), value);
        }
    }
    Ok((values, secrets))
}

async fn dev(args: DevArgs) -> anyhow::Result<()> {
    let found = locate(&args.path)?;
    let manifest = Manifest::parse(&found.manifest)?;
    let problems = manifest.problems();
    if !problems.is_empty() {
        for p in &problems {
            println!("  PROBLEM  manifest: {p}");
        }
        bail!("fix the manifest first; `niles plugin check` says the same");
    }
    let (path, component) = found
        .component
        .context("no .wasm found; build it with cargo build --release --target wasm32-wasip2")?;
    let (values, secrets) = dev_settings(&found.dir, &manifest, &args.settings)?;

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::new(1024);
    let storage = found.dir.join(".niles-dev").join("storage.json");
    let store: Arc<dyn PluginStore> = Arc::new(MemoryStore::in_file(storage.clone()));
    let house = House::new(registry.clone(), bus.clone(), store)
        .with_settings(Arc::new(move |_| values.clone()))
        .with_secrets(Arc::new(move |_, key| secrets.get(key).cloned()));
    let runtime = Runtime::new(house, Limits::default())?;
    let printer = spawn_printer(bus.subscribe(), registry.clone());

    println!(
        "{} {} from {}\nstorage: {}",
        manifest.name,
        manifest.version,
        path.display(),
        storage.display()
    );
    let id = manifest.id.clone();
    runtime.start(manifest, component).await?;
    let status = settled(&runtime, &id).await;
    println!("status: {}", status.line());
    print_tools(&runtime, &id);

    if let Some(call) = &args.call {
        let [tool, json] = call.as_slice() else {
            bail!("--call takes a tool and its JSON arguments");
        };
        call_tool(&runtime, &id, tool, json).await;
    }
    if let Some(surface) = &args.render {
        render(&runtime, &id, surface, None).await;
    }
    if !args.dry_run {
        println!(
            "Running; timers fire and changes print as they happen. Type `help` for commands, Ctrl-C to stop."
        );
        prompt(&runtime, &id, &registry, &bus).await;
    }
    runtime.shutdown().await;
    printer.abort();
    Ok(())
}

/// Wait for the plugin to finish starting, however that went.
async fn settled(runtime: &Runtime, id: &str) -> Status {
    loop {
        let status = runtime
            .info(id)
            .map(|i| i.status)
            .unwrap_or(Status::Starting);
        if status != Status::Starting {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn print_tools(runtime: &Runtime, id: &str) {
    let tools = runtime.tools(id);
    if tools.is_empty() {
        println!("tools: none");
    }
    for tool in tools {
        println!("tool {}: {}", tool.name, first_line(&tool.description));
    }
}

async fn call_tool(runtime: &Runtime, id: &str, tool: &str, json: &str) {
    match runtime.call_tool(id, tool, json).await {
        Ok(result) => println!("{result}"),
        Err(why) => println!("error: {why}"),
    }
}

async fn render(runtime: &Runtime, id: &str, surface: &str, room: Option<String>) {
    let context = Context { room, person: None };
    match runtime.render(id, surface, context).await {
        Ok(surface) => print!("{}", surface.text()),
        Err(why) => println!("error: {why}"),
    }
}

/// Print what the plugin does to the house as it happens.
fn spawn_printer(
    mut events: tokio::sync::broadcast::Receiver<Event>,
    registry: Arc<DeviceRegistry>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                Event::DeviceAdded { device } => println!(
                    "+ device {} ({:?}{}) {}",
                    device.id,
                    device.class,
                    if device.available {
                        ""
                    } else {
                        ", unavailable"
                    },
                    describe_state(&device.state)
                ),
                Event::DeviceStateChanged { id, state } => {
                    let available = registry.get(&id).is_none_or(|d| d.available);
                    let state = describe_state(&state);
                    match (state.is_empty(), available) {
                        (true, true) => println!("~ {id} available"),
                        (true, false) => println!("~ {id} unavailable"),
                        (false, true) => println!("~ {id} {state}"),
                        (false, false) => println!("~ {id} {state} (unavailable)"),
                    }
                }
                Event::DeviceAction { id, action } => println!("! {id} {action}"),
                Event::DeviceRemoved { id } => println!("- device {id}"),
                Event::UiInvalidated { surface, .. } => println!("~ page {surface} has changed"),
                _ => {}
            }
        }
    })
}

fn describe_state(state: &DeviceState) -> String {
    let mut parts = Vec::new();
    let mut add = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            parts.push(format!("{name}={value}"));
        }
    };
    add("on", state.on.map(|v| v.to_string()));
    add("brightness", state.brightness.map(|v| format!("{v}%")));
    add(
        "color_temp",
        state.color_temp_kelvin.map(|v| format!("{v}K")),
    );
    add("rgb", state.rgb.map(|[r, g, b]| format!("{r},{g},{b}")));
    add(
        "temperature",
        state.temperature_celsius.map(|v| format!("{v}°C")),
    );
    add("humidity", state.humidity_percent.map(|v| format!("{v}%")));
    add("battery", state.battery_percent.map(|v| format!("{v}%")));
    add("open", state.open.map(|v| v.to_string()));
    add("motion", state.motion.map(|v| v.to_string()));
    add(
        "illuminance",
        state.illuminance_lux.map(|v| format!("{v} lx")),
    );
    add("position", state.position.map(|v| format!("{v}%")));
    for (name, reading) in &state.readings {
        let unit = reading
            .unit
            .as_deref()
            .map(|u| format!(" {u}"))
            .unwrap_or_default();
        parts.push(format!("{name}={}{unit}", reading.value));
    }
    parts.join(" ")
}

const HELP: &str = "\
  tools                               list the tools as the model sees them
  tool <name> [json]                  call one, print the result
  set <room/name> <json>              call set-device, e.g. set office/lamp {\"on\":true}
  render <surface> [room]             draw a page or widget as text
  action <surface> <control> [json]   use a control; json is the value, {\"type\":\"pressed\"} if left out
  event device_state <room/name> <json>   as if a device elsewhere changed, e.g. event device_state office/door {\"open\":true}
  event device_action <room/name> <action>
  event presence home|away|unknown
  status                              how the plugin is doing
  restart                             stop, a fresh instance, init
  quit                                stop it and exit";

/// Something happening elsewhere in the stand-in house, from the
/// prompt, under the source `dev`; what was published, or why not.
fn house_event(line: &str, registry: &DeviceRegistry, bus: &EventBus) -> Result<String, String> {
    let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
    let (target, value) = rest.trim().split_once(' ').unwrap_or((rest.trim(), ""));
    let device = || {
        DeviceId::parse(&format!("dev:{target}")).map_err(|_| "a device is room/name".to_string())
    };
    let event = match kind {
        "device_state" => {
            let id = device()?;
            let state: DeviceState = serde_json::from_str(value).map_err(|e| e.to_string())?;
            if registry.get(&id).is_none() {
                registry.upsert(niles_core::Device::new(
                    id.clone(),
                    DeviceState::default(),
                    niles_core::DeviceClass::Sensor,
                ));
            }
            registry.merge_state(&id, state.clone());
            Event::DeviceStateChanged { id, state }
        }
        "device_action" if !value.is_empty() => Event::DeviceAction {
            id: device()?,
            action: value.to_string(),
        },
        "presence" => Event::PresenceChanged {
            state: match target {
                "home" => niles_core::PresenceState::Home,
                "away" => niles_core::PresenceState::Away,
                "unknown" => niles_core::PresenceState::Unknown,
                _ => return Err("presence is home, away or unknown".into()),
            },
        },
        _ => return Err("event device_state|device_action|presence ...; `help` shows how".into()),
    };
    let described = format!("{event:?}");
    bus.publish(event);
    Ok(described)
}

async fn prompt(runtime: &Runtime, id: &str, registry: &DeviceRegistry, bus: &EventBus) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    loop {
        let line = tokio::select! {
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(line)) => line,
                // No terminal: keep running until Ctrl-C.
                _ => { stdin_open = false; continue; }
            },
            _ = tokio::signal::ctrl_c() => return,
        };
        let line = line.trim();
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match command {
            "" => {}
            "help" => println!("{HELP}"),
            "quit" | "exit" => return,
            "tools" => print_tools(runtime, id),
            "event" => match house_event(rest, registry, bus) {
                Ok(_) => println!("sent"),
                Err(why) => println!("error: {why}"),
            },
            "status" => {
                if let Some(info) = runtime.info(id) {
                    println!("{}", info.status.line());
                }
            }
            "restart" => {
                let _ = runtime.restart(id);
                tokio::time::sleep(Duration::from_millis(100)).await;
                println!("status: {}", settled(runtime, id).await.line());
            }
            "tool" => {
                let (tool, json) = rest.split_once(' ').unwrap_or((rest, "{}"));
                call_tool(runtime, id, tool, json).await;
            }
            "set" => {
                let (device, json) = rest.split_once(' ').unwrap_or((rest, "{}"));
                let source = format!("x_{id}");
                let Ok(device) = DeviceId::parse(&format!("{source}:{device}")) else {
                    println!("error: a device is room/name");
                    continue;
                };
                match serde_json::from_str::<DeviceState>(json) {
                    Ok(state) => match runtime.command(&device, &state).await {
                        Ok(()) => println!("ok"),
                        Err(why) => println!("error: {why}"),
                    },
                    Err(e) => println!("error: {e}"),
                }
            }
            "render" => {
                let (surface, room) = rest.split_once(' ').unwrap_or((rest, ""));
                let room = Some(room.trim().to_string()).filter(|r| !r.is_empty());
                render(runtime, id, surface, room).await;
            }
            "action" => {
                let mut parts = rest.splitn(3, ' ');
                let (surface, control) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let value = parts.next().unwrap_or(r#"{"type":"pressed"}"#);
                match serde_json::from_str::<ActionValue>(value) {
                    Ok(value) => {
                        match runtime
                            .act(id, surface, control, value, Context::default())
                            .await
                        {
                            Ok(outcome) => println!(
                                "{}{}",
                                outcome.toast.as_deref().unwrap_or("ok"),
                                if outcome.rerender {
                                    " (render again)"
                                } else {
                                    ""
                                }
                            ),
                            Err(why) => println!("error: {why}"),
                        }
                    }
                    Err(e) => println!("error: {e}"),
                }
            }
            other => println!("no command {other:?}; `help` lists them"),
        }
    }
}

/// Where installed plugins are kept: the database, or nowhere a restart
/// keeps — plugins need the database, so without one none are loaded,
/// and none can be installed. True when it is the database.
fn plugin_store(cfg: &niles_config::Config) -> (Arc<dyn PluginStore>, bool) {
    let postgres = cfg.database.as_ref().and_then(|database| {
        let url = database.resolve_url().ok()?;
        let backend =
            niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
        Some(niles_db::PostgresPlugins::new(
            backend.pool(),
            backend.describe_target(),
        ))
    });
    match postgres {
        Some(store) => (Arc::new(store), true),
        None => {
            tracing::info!("[plugins] no database, so no plugins are installed");
            (Arc::new(MemoryStore::new()), false)
        }
    }
}

/// The runtime `serve` runs plugins in, and whether it keeps them in the
/// database, or `None` — logged, and Niles carries on without plugins —
/// if it cannot start.
pub(crate) fn start_runtime(
    cfg: &niles_config::Config,
    settings: Arc<niles_config::ConfigStore>,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    tools: niles_tools::DynamicTools,
) -> Option<(Runtime, bool)> {
    let (store, database) = plugin_store(cfg);
    let house = House::new(registry, bus, store)
        .with_tools(tools)
        .with_settings(Arc::new(move |id| {
            settings
                .current()
                .plugins
                .get(id)
                .cloned()
                .unwrap_or_default()
        }))
        .with_secrets(Arc::new(|id, key| {
            niles_config::secrets::get(&format!("plugins.{id}.{key}"))
        }));
    match Runtime::new(house, Limits::default()) {
        Ok(runtime) => Some((runtime, database)),
        Err(e) => {
            tracing::error!("[plugins] {e}; Niles runs without them");
            None
        }
    }
}

/// Restart a plugin whenever its settings change, as saving its setup
/// panel does: `stop`, a fresh instance, `init`.
pub(crate) fn restart_on_settings(
    runtime: Runtime,
    settings: Arc<niles_config::ConfigStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut changes = settings.subscribe();
        let mut last = settings.current().plugins.clone();
        while changes.changed().await.is_ok() {
            let now = settings.current().plugins.clone();
            for plugin in runtime.plugins() {
                if last.get(&plugin.id) != now.get(&plugin.id) {
                    tracing::info!("[plugins] {}'s settings changed; restarting it", plugin.id);
                    let _ = runtime.restart(&plugin.id);
                }
            }
            last = now;
        }
    })
}

/// What a removed plugin leaves elsewhere: its settings under
/// `[plugins.<id>]` and its pages and widgets under `[menu]` in the
/// config store, and its secrets under `plugins.<id>.*` in the secret
/// store.
pub(crate) struct Forget {
    pub settings: Arc<niles_config::ConfigStore>,
    pub secrets: Option<Arc<niles_db::PostgresSecrets>>,
}

#[async_trait::async_trait]
impl niles_plugins::Leftovers for Forget {
    async fn forget(&self, id: &str) -> Result<(), String> {
        let (menu, widgets) = self.settings.current().menu.forget_plugin(id);
        let mut patch = toml::Table::new();
        if !menu.is_empty() {
            patch.insert("menu".into(), toml::Value::Table(menu));
        }
        let mut resets = vec![format!("plugins.{id}")];
        if widgets {
            resets.push(format!("menu.widgets.{id}"));
        }
        // One revision, so one undo brings all of it back.
        self.settings
            .apply_and_reset(&patch, &resets, niles_config::ChangeSource::Api)
            .await
            .map_err(|e| format!("its settings and menu entries: {e}"))?;
        let Some(secrets) = &self.secrets else {
            return Ok(());
        };
        let keys = secrets.keys().await.map_err(|e| e.to_string())?;
        for key in secrets_of(id, &keys) {
            secrets.clear(key).await.map_err(|e| e.to_string())?;
        }
        let values = secrets.load_all().await.map_err(|e| e.to_string())?;
        niles_config::secrets::load(values);
        Ok(())
    }
}

/// The stored keys that are `id`'s: `plugins.<id>.<key>`, whether or
/// not its manifest still declares them.
fn secrets_of<'a>(id: &str, keys: &'a [String]) -> Vec<&'a str> {
    let prefix = format!("plugins.{id}.");
    keys.iter()
        .filter(|k| k.starts_with(&prefix))
        .map(String::as_str)
        .collect()
}

/// The plugins, as the HTTP API asks after them.
pub(crate) struct AppPlugins(pub niles_plugins::Installer);

fn summary(listed: niles_plugins::Listed) -> niles_api::plugins::PluginSummary {
    use niles_api::plugins::{PluginGroup, PluginSummary, PluginUpdate};
    PluginSummary {
        logo: listed
            .has_logo
            .then(|| format!("/plugins/{}/logo", listed.id)),
        id: listed.id,
        name: listed.name,
        version: listed.version,
        description: listed.description,
        authors: listed.authors,
        homepage: listed.homepage,
        group: PluginGroup {
            id: listed.group.id,
            label: listed.group.label,
        },
        status: listed.status,
        status_line: listed.status_line,
        error: listed.error,
        dropped_events: listed.dropped_events,
        enabled: listed.enabled,
        repo: listed.source.as_ref().map(|s| s.repo.clone()),
        release_tag: listed.source.as_ref().map(|s| s.tag.clone()),
        release_url: listed.source.map(|s| s.release_url),
        installed_at: listed.installed_at,
        updated_at: listed.updated_at,
        previous_version: listed.previous_version,
        update: listed.update.map(|u| PluginUpdate {
            version: u.version,
            tag: u.tag,
            release_url: u.release_url,
            notes: u.notes,
            added: u.added,
            removed: u.removed,
        }),
        settings: listed
            .settings
            .into_iter()
            .map(|s| niles_api::plugins::PluginSetting {
                key: s.key,
                label: s.label,
                kind: s.kind.to_string(),
                required: s.required,
                help: s.help,
                default: s.default,
                options: s
                    .options
                    .into_iter()
                    .map(|(value, label)| niles_api::plugins::PluginOption { value, label })
                    .collect(),
                min: s.min,
                max: s.max,
                value: s.value,
                set: s.set,
            })
            .collect(),
        menu: listed
            .menu
            .into_iter()
            .map(|m| niles_api::plugins::PluginPage {
                id: m.id,
                label: m.label,
                icon: m.icon,
                refresh_every: m.refresh_every,
            })
            .collect(),
        widgets: listed
            .widgets
            .into_iter()
            .map(|w| niles_api::plugins::PluginWidget {
                id: w.id,
                title: w.title,
                room: w.room,
                refresh_every: w.refresh_every,
            })
            .collect(),
        interface: listed.interface,
        older_interface: listed.older_interface,
    }
}

fn refused(refusal: niles_plugins::Refusal) -> PluginFailure {
    use niles_plugins::Refusal;
    match refusal {
        Refusal::BadRequest(s) => PluginFailure::BadRequest(s),
        Refusal::NotFound(s) => PluginFailure::NotFound(s),
        Refusal::Conflict(s) => PluginFailure::Conflict(s),
        Refusal::Invalid(s) => PluginFailure::Invalid(s),
        Refusal::Gone(s) => PluginFailure::Gone(s),
        Refusal::Upstream(s) => PluginFailure::Upstream(s),
        Refusal::Unavailable(s) => PluginFailure::Unavailable(s),
    }
}

fn as_json(preview: niles_plugins::Preview) -> Result<serde_json::Value, PluginFailure> {
    serde_json::to_value(preview).map_err(|e| PluginFailure::Unavailable(e.to_string()))
}

#[async_trait::async_trait]
impl niles_api::plugins::Plugins for AppPlugins {
    async fn list(&self) -> Vec<PluginSummary> {
        self.0.list().await.into_iter().map(summary).collect()
    }

    async fn render(
        &self,
        id: &str,
        surface: &str,
        viewer: niles_api::plugins::Viewer,
    ) -> Result<serde_json::Value, niles_api::plugins::SurfaceFailure> {
        let context = Context {
            room: viewer.room,
            person: viewer.person,
        };
        let surface = self
            .0
            .runtime()
            .render(id, surface, context)
            .await
            .map_err(failure)?;
        serde_json::to_value(surface)
            .map_err(|e| niles_api::plugins::SurfaceFailure::Refused(e.to_string()))
    }

    async fn act(
        &self,
        id: &str,
        surface: &str,
        control: &str,
        value: serde_json::Value,
        viewer: niles_api::plugins::Viewer,
    ) -> Result<serde_json::Value, niles_api::plugins::SurfaceFailure> {
        let value: ActionValue = serde_json::from_value(value).map_err(|e| {
            niles_api::plugins::SurfaceFailure::BadRequest(format!("not an action value: {e}"))
        })?;
        let context = Context {
            room: viewer.room,
            person: viewer.person,
        };
        let outcome = self
            .0
            .runtime()
            .act(id, surface, control, value, context)
            .await
            .map_err(failure)?;
        serde_json::to_value(outcome)
            .map_err(|e| niles_api::plugins::SurfaceFailure::Refused(e.to_string()))
    }

    async fn preview(
        &self,
        repo: &str,
        tag: Option<&str>,
    ) -> Result<serde_json::Value, PluginFailure> {
        as_json(self.0.preview(repo, tag).await.map_err(refused)?)
    }

    async fn install(
        &self,
        token: &str,
        sha256: Option<&str>,
    ) -> Result<PluginSummary, PluginFailure> {
        self.0
            .install(token, sha256)
            .await
            .map(summary)
            .map_err(refused)
    }

    async fn check_update(&self, id: &str) -> Result<PluginSummary, PluginFailure> {
        self.0.check_update(id).await.map_err(refused)?;
        self.0
            .list()
            .await
            .into_iter()
            .find(|p| p.id == id)
            .map(summary)
            .ok_or_else(|| PluginFailure::NotFound(format!("There is no plugin called {id}.")))
    }

    async fn update_preview(&self, id: &str) -> Result<serde_json::Value, PluginFailure> {
        as_json(self.0.update_preview(id).await.map_err(refused)?)
    }

    async fn update(
        &self,
        id: &str,
        token: &str,
        sha256: Option<&str>,
    ) -> Result<PluginSummary, PluginFailure> {
        self.0
            .update(id, token, sha256)
            .await
            .map(summary)
            .map_err(refused)
    }

    async fn roll_back(&self, id: &str) -> Result<PluginSummary, PluginFailure> {
        self.0.roll_back(id).await.map(summary).map_err(refused)
    }

    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<PluginSummary, PluginFailure> {
        let done = if enabled {
            self.0.enable(id).await
        } else {
            self.0.disable(id).await
        };
        done.map(summary).map_err(refused)
    }

    async fn remove(&self, id: &str) -> Result<(), PluginFailure> {
        self.0.remove(id).await.map_err(refused)
    }

    async fn logo(&self, id: &str) -> Result<Option<String>, PluginFailure> {
        self.0.logo(id).await.map_err(refused)
    }

    async fn log(&self, id: &str) -> Result<Vec<niles_api::plugins::PluginLogLine>, PluginFailure> {
        if !self.0.list().await.iter().any(|p| p.id == id) {
            return Err(PluginFailure::NotFound(format!(
                "There is no plugin called {id}."
            )));
        }
        Ok(self
            .0
            .runtime()
            .log(id)
            .into_iter()
            .map(|line| niles_api::plugins::PluginLogLine {
                at: line.at,
                level: match line.level {
                    niles_plugins::log::Level::Debug => "debug",
                    niles_plugins::log::Level::Info => "info",
                    niles_plugins::log::Level::Warn => "warn",
                    niles_plugins::log::Level::Error => "error",
                }
                .to_string(),
                message: line.message,
                niles: line.niles,
            })
            .collect())
    }

    async fn image(
        &self,
        id: &str,
        src: &str,
    ) -> Result<niles_api::plugins::PluginImage, PluginFailure> {
        use niles_plugins::images::ImageError;
        match self.0.runtime().image(id, src).await {
            Ok(image) => Ok(niles_api::plugins::PluginImage {
                mime: image.mime.to_string(),
                bytes: image.bytes,
            }),
            Err(ImageError::NotAllowed(why)) => Err(PluginFailure::BadRequest(why)),
            Err(ImageError::NotFound(why)) => Err(PluginFailure::NotFound(why)),
            Err(ImageError::Upstream(why)) => Err(PluginFailure::Upstream(why)),
        }
    }

    async fn secret_settings(&self) -> Vec<(String, String)> {
        self.0.secret_settings().await
    }

    fn restart(&self, id: &str) {
        self.0.restart(id);
    }
}

fn failure(error: SurfaceError) -> niles_api::plugins::SurfaceFailure {
    match error {
        SurfaceError::NotFound(why) => niles_api::plugins::SurfaceFailure::NotFound(why),
        SurfaceError::Plugin(why) => niles_api::plugins::SurfaceFailure::Refused(why),
        SurfaceError::Unavailable(why) => niles_api::plugins::SurfaceFailure::Unavailable(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
id = "spotprice"
name = "Spot prices"
version = "0.1.0"
description = "Prices."
authors = ["Jane"]
min_niles = "0.2.0"
interface = "0.1"

[[settings]]
key = "area"
label = "Area"
type = "choice"
options = [{ value = "DK1", label = "DK1" }, { value = "DK2", label = "DK2" }]

[[settings]]
key = "cheap_hours"
label = "Cheap hours"
type = "number"

[[settings]]
key = "api_key"
label = "API key"
type = "secret"
"#;

    #[test]
    fn dev_settings_come_from_the_file_and_the_flags_typed_by_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("dev-settings.toml"),
            "area = \"DK1\"\napi_key = \"hunter2\"\n",
        )
        .unwrap();
        let manifest = Manifest::parse(MANIFEST).unwrap();
        let (values, secrets) = dev_settings(
            dir.path(),
            &manifest,
            &["area=DK2".into(), "cheap_hours=6".into()],
        )
        .unwrap();
        assert_eq!(values["area"].as_str(), Some("DK2"), "the flag wins");
        assert_eq!(values["cheap_hours"].as_float(), Some(6.0));
        assert_eq!(secrets["api_key"], "hunter2");
        assert!(
            !values.contains_key("api_key"),
            "a secret is never a setting"
        );

        assert!(dev_settings(dir.path(), &manifest, &["area=DK3".into()]).is_err());
        assert!(dev_settings(dir.path(), &manifest, &["nope=1".into()]).is_err());
    }

    #[test]
    fn a_wasm_finds_its_manifest_above_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("niles-integration.toml"), MANIFEST).unwrap();
        let release = dir
            .path()
            .join("target")
            .join("wasm32-wasip2")
            .join("release");
        std::fs::create_dir_all(&release).unwrap();
        let wasm = release.join("niles_spotprice.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();

        let from_wasm = locate(&wasm).unwrap();
        assert_eq!(from_wasm.dir, dir.path());
        let from_dir = locate(dir.path()).unwrap();
        assert_eq!(from_dir.component.unwrap().0, wasm);
    }

    #[tokio::test]
    async fn the_prompt_can_make_things_happen_in_the_house() {
        let registry = DeviceRegistry::new();
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        house_event(r#"device_state office/door {"open":true}"#, &registry, &bus).unwrap();
        match events.recv().await.unwrap() {
            Event::DeviceStateChanged { id, state } => {
                assert_eq!(id.to_string(), "dev:office/door");
                assert_eq!(state.open, Some(true));
            }
            other => panic!("{other:?}"),
        }
        house_event("device_action hall/button single", &registry, &bus).unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            Event::DeviceAction { .. }
        ));
        house_event("presence away", &registry, &bus).unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            Event::PresenceChanged { .. }
        ));
        assert!(house_event("presence sideways", &registry, &bus).is_err());
        assert!(house_event("weather sunny", &registry, &bus).is_err());
    }

    #[test]
    fn a_plugins_secrets_are_the_keys_under_its_id() {
        let keys: Vec<String> = [
            "plugins.hello.token",
            "plugins.hello.old_key",
            "plugins.hello2.token",
            "plugins.hellox",
            "mqtt.password",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            secrets_of("hello", &keys),
            vec!["plugins.hello.token", "plugins.hello.old_key"]
        );
    }

    #[tokio::test]
    async fn removing_a_plugin_forgets_its_settings_and_only_its() {
        use niles_plugins::Leftovers;
        let settings =
            Arc::new(niles_config::ConfigStore::from_str_in_memory("").expect("an empty config"));
        let patch: toml::Table = toml::from_str(
            "[plugins.hello]\ngreeting = \"Hej\"\n[plugins.spotprice]\narea = \"DK2\"\n",
        )
        .unwrap();
        settings
            .apply(&patch, niles_config::ChangeSource::Api)
            .await
            .unwrap();
        let forget = Forget {
            settings: settings.clone(),
            secrets: None,
        };
        forget.forget("hello").await.unwrap();
        let now = settings.current();
        assert!(!now.plugins.contains_key("hello"));
        assert_eq!(now.plugins["spotprice"]["area"].as_str(), Some("DK2"));
        forget.forget("never_installed").await.unwrap();
    }

    #[tokio::test]
    async fn removing_a_plugin_takes_its_pages_and_widgets_out_of_the_menu() {
        use niles_plugins::Leftovers;
        let settings =
            Arc::new(niles_config::ConfigStore::from_str_in_memory("").expect("an empty config"));
        let patch: toml::Table = toml::from_str(
            "[menu]\norder = [\"chat\", \"hello.page\", \"spotprice.electricity\"]\n\
             hidden = [\"hello.other\"]\n\
             [menu.widgets.hello]\ncard = \"front\"\n\
             [menu.widgets.spotprice]\nnow = \"room:kitchen\"\n",
        )
        .unwrap();
        settings
            .apply(&patch, niles_config::ChangeSource::Api)
            .await
            .unwrap();
        let forget = Forget {
            settings: settings.clone(),
            secrets: None,
        };
        forget.forget("hello").await.unwrap();
        let menu = &settings.current().menu;
        assert_eq!(menu.order, ["chat", "spotprice.electricity"]);
        assert!(menu.hidden.is_empty());
        assert!(!menu.widgets.contains_key("hello"));
        assert_eq!(menu.widgets["spotprice"]["now"], "room:kitchen");
    }

    /// What the setup panel will do: write `[plugins.<id>]` through
    /// `PATCH /config`'s store, and see the plugin start again with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_settings_change_restarts_the_plugin_with_the_new_values() {
        let settings =
            Arc::new(niles_config::ConfigStore::from_str_in_memory("").expect("an empty config"));
        let read = settings.clone();
        let house = House::new(
            Arc::new(DeviceRegistry::new()),
            EventBus::default(),
            Arc::new(MemoryStore::new()),
        )
        .with_settings(Arc::new(move |id| {
            read.current().plugins.get(id).cloned().unwrap_or_default()
        }));
        let runtime = Runtime::new(house, Limits::default()).unwrap();
        runtime
            .install(
                include_str!("../../niles-plugins/tests/fixtures/hello/niles-integration.toml"),
                include_bytes!("../../niles-plugins/tests/fixtures/hello.wasm").to_vec(),
            )
            .await
            .unwrap();
        let _restarts = restart_on_settings(runtime.clone(), settings.clone());
        let greeting = || async {
            let until = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                if let Ok(text) = runtime.call_tool("hello", "greet", "{}").await {
                    return text;
                }
                assert!(std::time::Instant::now() < until, "hello never answered");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        assert_eq!(greeting().await, "Hello, world!");

        let patch: toml::Table = toml::from_str("[plugins.hello]\ngreeting = \"Hej\"\n").unwrap();
        settings
            .apply(&patch, niles_config::ChangeSource::Api)
            .await
            .unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(20);
        while greeting().await != "Hej, world!" {
            assert!(std::time::Instant::now() < until, "never restarted");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        runtime.shutdown().await;
    }

    #[test]
    fn state_prints_compactly_with_units() {
        let mut state = DeviceState {
            on: Some(true),
            brightness: Some(60),
            ..Default::default()
        };
        state.readings.insert(
            "power".into(),
            niles_core::Reading::new(340.0, Some("W".into())),
        );
        assert_eq!(describe_state(&state), "on=true brightness=60% power=340 W");
    }
}

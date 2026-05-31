//! niles — AI-first home automation system.

use anyhow::Context;
use chrono::{Timelike, Utc};
use clap::{Args, Parser, Subcommand};
use niles_api::{AppState, DevicePublisher};
use niles_capabilities::CapabilityLoader;
use niles_config::Config;
use niles_core::{Device, DeviceId, DeviceRegistry, DeviceState, Event, EventBus, RoomName};
use niles_history::{
    CommandEntry, CommandReader, CommandWriter, StateEntry, StateReader, StateWriter,
};
use niles_intent::{DeviceIndex, Intent, IntentRouter, RouterContext};
use niles_llm::{
    ChatRequest, ChatResponse, GroqClient, GroqConfig, Message, OpenAiClient, OpenAiConfig,
    ToolChoice,
};
use niles_memory::MemoryStore;
use niles_mqtt::{
    MqttClient, MqttOptions, MqttPublisher, Z2mSource, format_set_command, is_actionable,
};
use niles_notifications::NotificationCenter;
use niles_scheduler::{
    BRIGHTNESS_DEBOUNCE, ManualModeTracker, MinuteOfDay, MorningClaimTracker, MorningRoutineConfig,
    SceneStore, SwitchEffect, TimerEntry, TimerStore, brightness_at, build_curve_target,
    classify_action, color_temp_at, routine_brightness_at, should_fire_today,
};
use niles_skills::{SkillStatus, SkillStore, SkillSummary};
use niles_speakers::SonosClient;
use niles_stt::{PcmFormat, WhisperClient, WhisperConfig, pcm_to_wav};
use niles_tools::{LookUpCapability, ToolRegistry};
use niles_tts::{PiperClient, PiperConfig};
use niles_wyoming::{SessionTracker, WyomingSender, WyomingServer};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

mod manifest;
mod response;
mod review;
mod satellites;
mod speak;
mod speakers;

use manifest::{GenerateManifestArgs, generate_manifest};
use satellites::SatelliteRegistry;

#[derive(Parser)]
#[command(name = "niles", about = "AI-first home automation system", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full Niles stack in one process: Z2M source, HTTP API,
    /// voice dispatch (Wyoming + STT + intent), and the ambient-lighting
    /// curve, all sharing one device registry and one manual-mode tracker.
    Serve(ServeArgs),
    /// One-shot helper to migrate state from Home Assistant.
    MigrateFromHa,
    /// Flash firmware to a satellite device.
    FlashSatellite,
    /// Configuration utilities.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// LLM tools registry utilities.
    Tools {
        #[command(subcommand)]
        action: ToolsAction,
    },
    /// Connect to MQTT and print received messages (for development).
    MqttTap(MqttTapArgs),
    /// Discover devices via Z2M and print add/remove/state-change events.
    Discover(DiscoverArgs),
    /// Send a Z2M set-command to a device (dev tool).
    Set(SetArgs),
    /// Run the Z2M source + HTTP API together. Devices land in the
    /// registry as Z2M reports them, and `curl http://<bind>/devices`
    /// returns the current snapshot.
    Api(ApiArgs),
    /// Run the Wyoming server and log every incoming event from
    /// connected satellites (dev tool — no STT / TTS yet).
    WyomingTap(WyomingTapArgs),
    /// Transcribe an audio file via the configured STT provider
    /// (Groq Whisper). The file is uploaded as-is — supported
    /// formats are WAV, MP3, FLAC, OGG, M4A, etc.
    Transcribe(TranscribeArgs),
    /// Synthesize speech via the configured TTS provider (Piper).
    /// Writes the resulting WAV to the path given by `--out`.
    Synthesize(SynthesizeArgs),
    /// One-shot chat with the configured Tier 1 LLM (Groq GPT-OSS by
    /// default). Sends `prompt` as a single user message with no tools
    /// and prints the model's reply to stdout. Manual-verification
    /// path — voice loop integration comes in a later PR.
    Chat(ChatArgs),
    /// Run the Wyoming server, accumulate each satellite utterance
    /// between `audio-start` and `audio-stop`, send it to the STT
    /// provider, and print the transcript. (Dev tool — no intent
    /// dispatch or TTS yet.)
    VoiceTap(VoiceTapArgs),
    /// Same audio pipeline as `voice-tap`, plus parse each
    /// transcript through the Tier 0 intent router and publish MQTT
    /// `set` commands for matched intents. Connects to MQTT and
    /// populates the device registry from Z2M, so room references
    /// like "kitchen" resolve to real devices. Pass `--dry-run` to
    /// match and log without publishing.
    VoiceDispatch(VoiceDispatchArgs),
    /// Run the ambient-lighting curve driver: every minute, compute
    /// the curve's brightness + color-temperature target and publish
    /// it to every currently-on light. Per the architecture the curve
    /// never turns lights on or off — it only adjusts ones already
    /// on. In standalone mode no flags are set, so the curve runs for
    /// all on-lights; use `niles serve` for manual-mode integration.
    Lighting(LightingArgs),
    /// Regenerate MANIFEST.md from features.toml. Use --check to
    /// verify the committed MANIFEST.md is in sync (CI mode).
    GenerateManifest(GenerateManifestArgs),
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Validate the Niles config file.
    Validate,
}

#[derive(Subcommand)]
enum ToolsAction {
    /// List registered LLM tools.
    List,
}

#[derive(Args)]
struct MqttTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// MQTT topic to subscribe to (wildcards `+` and `#` supported).
    #[arg(short, long, default_value = "zigbee2mqtt/#")]
    topic: String,
}

#[derive(Args)]
struct DiscoverArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
}

#[derive(Args)]
struct SetArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Target device as "<room>/<device>" (source prefix `z2m:` is added).
    device: String,
    /// Turn on (`--on true`), off (`--on false`), or leave unchanged.
    #[arg(long)]
    on: Option<bool>,
    /// Brightness percent (0–100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    brightness: Option<u8>,
    /// Color temperature in Kelvin (typical lighting range 1000–20000).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1000..=20000))]
    kelvin: Option<u16>,
    /// Print the message that would be published without actually sending.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct ApiArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
}

#[derive(Args)]
struct WyomingTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
}

#[derive(Args)]
struct ChatArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Prompt to send as a single user message.
    prompt: String,
}

#[derive(Args)]
struct TranscribeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Audio file to transcribe (WAV, MP3, FLAC, OGG, M4A, ...).
    audio: PathBuf,
}

#[derive(Args)]
struct SynthesizeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Text to synthesize.
    text: String,
    /// Path to write the resulting WAV file.
    #[arg(short, long)]
    out: PathBuf,
    /// Override the default voice from the [tts] config.
    #[arg(long)]
    voice: Option<String>,
}

#[derive(Args)]
struct VoiceTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
}

#[derive(Args)]
struct VoiceDispatchArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Match and log intents but skip the actual MQTT publish.
    /// Useful for confirming the audio + intent pipeline against a
    /// real broker-populated registry without affecting devices.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct LightingArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Compute and log the curve target each tick, but don't publish.
    #[arg(long)]
    dry_run: bool,
    /// Override the default 60-second tick interval. Mostly useful
    /// for development — the curve is integer-minute discretized,
    /// so polling faster than once a minute doesn't change values.
    #[arg(long, default_value_t = 60)]
    tick_seconds: u64,
}

#[derive(Args)]
struct ServeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Match and log intents but skip the actual MQTT publish.
    #[arg(long)]
    dry_run: bool,
    /// Override the default 60-second curve tick interval.
    #[arg(long, default_value_t = 60)]
    tick_seconds: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env at repo root for local dev. Silently ignored if absent.
    let _ = dotenvy::dotenv();

    // Initialize tracing. Honors RUST_LOG; defaults to `info` for the
    // niles_* crates so dev subcommands give meaningful output.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("niles_mqtt=info,niles_bin=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Serve(args) => serve(args).await,
        Commands::MigrateFromHa => todo!("implement `migrate-from-ha`"),
        Commands::FlashSatellite => todo!("implement `flash-satellite`"),
        Commands::Config { action } => match action {
            ConfigAction::Validate => config_validate().await,
        },
        Commands::Tools { action } => match action {
            ToolsAction::List => todo!("implement `tools list`"),
        },
        Commands::MqttTap(args) => mqtt_tap(args).await,
        Commands::Discover(args) => discover(args).await,
        Commands::Set(args) => set(args).await,
        Commands::Api(args) => api(args).await,
        Commands::WyomingTap(args) => wyoming_tap(args).await,
        Commands::Transcribe(args) => transcribe(args).await,
        Commands::Synthesize(args) => synthesize(args).await,
        Commands::Chat(args) => chat(args).await,
        Commands::VoiceTap(args) => voice_tap(args).await,
        Commands::VoiceDispatch(args) => voice_dispatch(args).await,
        Commands::Lighting(args) => lighting(args).await,
        Commands::GenerateManifest(args) => generate_manifest(args),
    }
}

async fn config_validate() -> anyhow::Result<()> {
    todo!("implement `config validate`")
}

/// Build a `HashSet<DeviceId>` from the `[ambient_lights]` config
/// section. Devices listed here are excluded from the ambient
/// lighting curve and the morning routine.
fn build_ambient_set(cfg: &Config) -> Arc<HashSet<DeviceId>> {
    let mut set = HashSet::new();
    for raw in &cfg.ambient_lights.devices {
        match DeviceId::parse(&format!("z2m:{raw}")) {
            Ok(id) => {
                set.insert(id);
            }
            Err(e) => {
                tracing::warn!("ambient_lights device {raw:?} failed to parse: {e}");
            }
        }
    }
    Arc::new(set)
}

/// Build an `MqttClient` connected to the broker described in
/// `niles.toml`, with credentials resolved from env vars.
async fn connect_from_config(config_path: &Path) -> anyhow::Result<(Config, MqttClient)> {
    let cfg = Config::load_from_path(config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    cfg.validate().context("validating config")?;

    let (username, password) = cfg
        .mqtt
        .resolve_credentials()
        .context("resolving MQTT credentials from environment")?;

    eprintln!(
        "Connecting to mqtt://{}:{} as client_id={} ...",
        cfg.mqtt.host, cfg.mqtt.port, cfg.mqtt.client_id
    );

    let opts = MqttOptions::new(&cfg.mqtt.host, cfg.mqtt.port, &cfg.mqtt.client_id)
        .with_credentials(username, password);
    let client = MqttClient::connect(opts);
    Ok((cfg, client))
}

async fn mqtt_tap(args: MqttTapArgs) -> anyhow::Result<()> {
    let (_cfg, mut client) = connect_from_config(&args.config).await?;

    client
        .subscribe(&args.topic)
        .await
        .with_context(|| format!("subscribing to '{}'", args.topic))?;

    eprintln!("Subscribed to '{}'. Press Ctrl-C to exit.\n", args.topic);

    loop {
        tokio::select! {
            msg = client.next_message() => match msg {
                Some(msg) => {
                    let body = msg.payload_str().unwrap_or("<binary payload>");
                    println!("[{}] {}", msg.topic, body);
                }
                None => {
                    let reason = client.last_error().await;
                    match reason {
                        Some(r) => eprintln!("\nMQTT event loop terminated: {r}"),
                        None => eprintln!("\nMQTT event loop terminated."),
                    }
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    Ok(())
}

async fn discover(args: DiscoverArgs) -> anyhow::Result<()> {
    let (cfg, client) = connect_from_config(&args.config).await?;

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let mut bus_rx = bus.subscribe();

    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        client,
        registry.clone(),
        bus.clone(),
        &cfg.mqtt.z2m_prefix,
        ambient_set,
    );

    eprintln!(
        "Subscribed to {prefix}/bridge/devices and {prefix}/+/+. Press Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix
    );

    let source_handle = tokio::spawn(async move { source.run().await });

    loop {
        tokio::select! {
            ev = bus_rx.recv() => match ev {
                Ok(Event::DeviceAdded { device }) => println!("+ {}", device.id),
                Ok(Event::DeviceRemoved { id }) => println!("- {id}"),
                Ok(Event::DeviceStateChanged { id, state }) => {
                    println!("~ {id} {state:?}");
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("(skipped {n} events — receiver fell behind)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C.");
                break;
            }
        }
    }

    source_handle.abort();

    let devices = registry.list_all();
    println!("\nFinal registry ({} devices):", devices.len());
    for d in devices {
        println!("  {} state={:?}", d.id, d.state);
    }

    Ok(())
}

async fn set(args: SetArgs) -> anyhow::Result<()> {
    let id = DeviceId::parse(&format!("z2m:{}", args.device))
        .with_context(|| format!("parsing device {:?}", args.device))?;

    let target = DeviceState {
        on: args.on,
        brightness: args.brightness,
        color_temp_kelvin: args.kelvin,
        ..Default::default()
    };
    if !is_actionable(&target) {
        anyhow::bail!("nothing to set — pass at least one of --on / --brightness / --kelvin");
    }

    if args.dry_run {
        // Dry-run uses the config's z2m_prefix if available, but doesn't
        // need credentials or a connection. Default to "zigbee2mqtt"
        // if the config can't be loaded so the user can preview a
        // command without a working config.
        let prefix = Config::load_from_path(&args.config)
            .map(|cfg| cfg.mqtt.z2m_prefix)
            .unwrap_or_else(|_| "zigbee2mqtt".into());
        let (topic, payload) = format_set_command(&prefix, &id, &target);
        println!("[dry-run] {topic}");
        println!("[dry-run] {payload}");
        return Ok(());
    }

    let (cfg, client) = connect_from_config(&args.config).await?;
    let (topic, payload) = format_set_command(&cfg.mqtt.z2m_prefix, &id, &target);
    println!("Publishing {topic}");
    println!("          {payload}");
    client.publish(&topic, payload).await?;

    // QoS::AtLeastOnce: publish() returns once the message is queued,
    // not once it's confirmed on the wire. Give the event loop a
    // beat to flush before we drop the client (which aborts the
    // event-loop task). 250ms is overkill for a healthy broker.
    tokio::time::sleep(Duration::from_millis(250)).await;

    Ok(())
}

async fn api(args: ApiArgs) -> anyhow::Result<()> {
    let (cfg, client) = connect_from_config(&args.config).await?;
    let bind = cfg
        .api
        .socket_addr()
        .context("resolving api.bind_address")?;

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();

    let publisher = client.publisher();
    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        client,
        registry.clone(),
        bus.clone(),
        &cfg.mqtt.z2m_prefix,
        ambient_set,
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    let state = AppState::new(
        registry.clone(),
        Arc::new(publisher) as Arc<dyn DevicePublisher>,
        Arc::new(cfg.mqtt.z2m_prefix.clone()),
        bus.clone(),
    );
    let api_handle = tokio::spawn(async move {
        if let Err(e) = niles_api::serve(bind, state).await {
            tracing::error!("API server exited: {e}");
        }
    });

    eprintln!(
        "Z2M source running on {prefix}/+/+; API listening on http://{bind}\n  GET  /devices   /rooms/<room>   /healthz\n  WS   /events/stream\n  POST /rooms/<room>/<device>\nPress Ctrl-C to exit.",
        prefix = cfg.mqtt.z2m_prefix
    );

    tokio::signal::ctrl_c()
        .await
        .context("listening for Ctrl-C")?;
    eprintln!("\nReceived Ctrl-C, shutting down.");

    api_handle.abort();
    source_handle.abort();
    Ok(())
}

async fn wyoming_tap(args: WyomingTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    // wyoming-tap doesn't track sessions, so disconnect events
    // have nothing to clean up — discard the receiver. The server
    // sends best-effort and won't block on a dropped receiver.
    let (server, mut rx, _disconnects) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nPoint your satellite at it. Press Ctrl-C to exit.\n"
    );

    let server_handle = tokio::spawn(server.run());

    loop {
        tokio::select! {
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    let kind = incoming.event.kind.as_wire_str();
                    let payload_len = incoming.event.payload.len();
                    if payload_len > 0 {
                        println!("[{}] {kind} (+ {payload_len}B payload)", incoming.from);
                    } else {
                        println!("[{}] {kind} {data}", incoming.from, data = incoming.event.data);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    server_handle.abort();
    Ok(())
}

/// Build a `WhisperClient` from the `[stt]` section of an already-
/// validated config, resolving the API key from the environment.
fn build_whisper_client(cfg: &Config) -> anyhow::Result<WhisperClient> {
    let api_key = cfg
        .stt
        .resolve_api_key()
        .context("resolving STT API key from environment")?;
    let whisper_cfg = WhisperConfig {
        api_key,
        base_url: cfg.stt.base_url.clone(),
        model: cfg.stt.model.clone(),
        language: cfg.stt.language.clone(),
        request_timeout: Duration::from_secs(cfg.stt.timeout_seconds),
    };
    WhisperClient::new(whisper_cfg).context("building Whisper HTTP client")
}

/// Build a `GroqClient` from the `[llm]` section of an already-
/// validated config, resolving the API key from the environment.
fn build_groq_client(cfg: &Config) -> anyhow::Result<GroqClient> {
    let api_key = cfg
        .llm
        .resolve_api_key()
        .context("resolving LLM API key from environment")?;
    let groq_cfg = GroqConfig {
        api_key,
        base_url: cfg.llm.base_url.clone(),
        model: cfg.llm.model.clone(),
        request_timeout: Duration::from_secs(cfg.llm.timeout_seconds),
    };
    GroqClient::new(groq_cfg).context("building Groq HTTP client")
}

/// Build an OpenAI client for Tier 2, or `None` when `[llm.tier2]` is absent.
fn build_tier2_client(cfg: &Config) -> anyhow::Result<Option<Arc<dyn ChatProvider>>> {
    let Some(tier2_cfg) = &cfg.llm.tier2 else {
        return Ok(None);
    };
    let api_key = tier2_cfg
        .resolve_api_key()
        .context("resolving Tier 2 API key from environment")?;
    let openai_cfg = OpenAiConfig {
        api_key,
        base_url: tier2_cfg.base_url.clone(),
        model: tier2_cfg.model.clone(),
        request_timeout: Duration::from_secs(tier2_cfg.timeout_seconds),
    };
    let client = OpenAiClient::new(openai_cfg).context("building OpenAI HTTP client")?;
    Ok(Some(Arc::new(client)))
}

/// Build a `CapabilityLoader` from config, or `None` if capabilities
/// are not configured or fail to load. A loader failure is **not
/// fatal** — niles starts without the `look_up_capability` tool and
/// logs the cause. This matches the optional-subsystem semantics of
/// the voice / STT / TTS stacks.
fn build_capability_loader(
    cfg: &niles_config::CapabilitiesConfig,
) -> Option<Arc<CapabilityLoader>> {
    let Some(dir) = &cfg.directory else {
        tracing::info!(
            "no capabilities directory configured; LLM will run without look_up_capability"
        );
        return None;
    };
    match CapabilityLoader::load_from_dir(dir) {
        Ok(loader) if loader.is_empty() => {
            tracing::warn!(
                "capability directory {} is empty; not registering look_up_capability",
                dir.display()
            );
            None
        }
        Ok(loader) => {
            tracing::info!(
                "loaded {} capabilities from {}",
                loader.len(),
                dir.display()
            );
            Some(Arc::new(loader))
        }
        Err(e) => {
            tracing::warn!("failed to load capabilities from {}: {e}", dir.display());
            None
        }
    }
}

fn build_capability_index(loader: &CapabilityLoader) -> niles_intent::CapabilityIndex {
    let entries = loader
        .iter()
        .map(|cap| niles_intent::CapabilityIndexEntry {
            name: cap.metadata.name.clone(),
            description: cap.metadata.description.clone(),
            prerequisites: cap.metadata.prerequisites.clone(),
        })
        .collect();
    niles_intent::CapabilityIndex::from_entries(entries)
}

fn build_initial_device_index(registry: &DeviceRegistry) -> DeviceIndex {
    let mut idx = DeviceIndex::new();
    for device in registry.list_all() {
        if device.is_light() {
            idx.insert(device.id.clone());
        }
    }
    idx
}

fn origin_context(room: &RoomName) -> String {
    format!(
        "\n\n# Current context\n\nThe user is speaking from a device in **{}**. \
         When a request is ambiguous between rooms, prefer devices in this room.\n",
        room.as_str(),
    )
}

pub(crate) fn home_context(home: &niles_config::HomeConfig) -> String {
    fn prompt_field(value: &str) -> String {
        value
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    let units_label = match home.resolved_units() {
        niles_config::Units::Metric => "metric (°C, km)",
        niles_config::Units::Imperial => "imperial (°F, miles)",
    };
    let country = home.resolved_country();
    let language = home.resolved_language();
    let name = prompt_field(&home.name);
    let locale = prompt_field(&home.locale);
    let timezone = prompt_field(&home.timezone);
    let country = country
        .as_deref()
        .map(prompt_field)
        .unwrap_or_else(|| "unknown".to_string());
    let language = prompt_field(&language);
    format!(
        "\n\n# Household context\n\n\
         - Home: {name}\n\
         - Country: {country}\n\
         - Locale: {locale}\n\
         - Timezone: {tz}\n\
         - Units: {units}\n\
         - Spoken language: {lang}\n",
        name = name,
        country = country,
        locale = locale,
        tz = timezone,
        units = units_label,
        lang = language,
    )
}

#[cfg(test)]
fn persona_with_origin(origin_room: Option<&RoomName>) -> String {
    let mut out = NILES_SYSTEM_PERSONA.to_string();
    if let Some(room) = origin_room {
        out.push_str(&origin_context(room));
    }
    out
}

/// Join memory entries into a single newline-separated string.
/// Returns `None` if the result is empty or whitespace-only.
fn join_memory_entries(entries: &[niles_memory::Entry]) -> Option<String> {
    let s = entries
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if s.trim().is_empty() { None } else { Some(s) }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn assemble_system_prompt(
    transcript: &str,
    home: &niles_config::HomeConfig,
    index: &niles_intent::CapabilityIndex,
    loader: &CapabilityLoader,
    origin_room: Option<&RoomName>,
    user_mem: Option<&str>,
    agent_mem: Option<&str>,
    skill_summaries: Option<&[SkillSummary]>,
) -> String {
    assemble_system_prompt_with_optional_capabilities(
        transcript,
        home,
        Some(index),
        Some(loader),
        origin_room,
        user_mem,
        agent_mem,
        skill_summaries,
    )
}

fn append_capability_references(
    out: &mut String,
    transcript: &str,
    index: &niles_intent::CapabilityIndex,
    loader: &CapabilityLoader,
) {
    let names = niles_intent::detect_topics(transcript, index);
    if names.is_empty() {
        return;
    }

    out.push_str(
        "\n\n# Capability references\n\nThe following references \
         are relevant to the current request:\n",
    );
    for name in &names {
        if let Some(cap) = loader.get(name) {
            out.push_str(&format!(
                "\n## {} (v{})\n{}\n",
                cap.metadata.name, cap.metadata.version, cap.body,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble_system_prompt_with_optional_capabilities(
    transcript: &str,
    home: &niles_config::HomeConfig,
    capability_index: Option<&niles_intent::CapabilityIndex>,
    capability_loader: Option<&CapabilityLoader>,
    origin_room: Option<&RoomName>,
    user_mem: Option<&str>,
    agent_mem: Option<&str>,
    skill_summaries: Option<&[SkillSummary]>,
) -> String {
    let mut out = String::from(NILES_SYSTEM_PERSONA);
    out.push_str(&home_context(home));

    if let Some(mem) = user_mem
        && !mem.trim().is_empty()
    {
        out.push_str("\n\n# User memory\n\n");
        out.push_str(mem);
    }
    if let Some(mem) = agent_mem
        && !mem.trim().is_empty()
    {
        out.push_str("\n\n# Agent memory\n\n");
        out.push_str(mem);
    }
    if let Some(summaries) = skill_summaries
        && !summaries.is_empty()
    {
        render_skills_section(&mut out, summaries);
    }

    if let (Some(index), Some(loader)) = (capability_index, capability_loader) {
        append_capability_references(&mut out, transcript, index, loader);
    }
    if let Some(room) = origin_room {
        out.push_str(&origin_context(room));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_tool_registry(
    registry: Arc<DeviceRegistry>,
    publisher: MqttPublisher,
    z2m_prefix: Arc<String>,
    capability_loader: Option<Arc<CapabilityLoader>>,
    memory_store: Option<Arc<MemoryStore>>,
    skill_store: Option<Arc<SkillStore>>,
    weather_client: Option<Arc<niles_weather::OpenMeteoClient>>,
    websearch_client: Option<Arc<niles_websearch::SearXngClient>>,
    websearch_default_num_results: u8,
    home: &niles_config::HomeConfig,
) -> ToolRegistry {
    let mut tools = niles_tools::default_registry(registry, publisher, z2m_prefix);
    if let Some(loader) = capability_loader {
        tools.register(Box::new(LookUpCapability::new(loader)));
    }
    if let Some(store) = memory_store {
        niles_tools::register_memory_tools(&mut tools, store);
    }
    if let Some(store) = skill_store {
        niles_tools::register_skill_tools(&mut tools, store);
    }
    if let Some(client) = weather_client {
        let units = match home.resolved_units() {
            niles_config::Units::Metric => niles_weather::Units::Metric,
            niles_config::Units::Imperial => niles_weather::Units::Imperial,
        };
        niles_tools::register_weather_tools(
            &mut tools,
            client,
            home.latitude,
            home.longitude,
            units,
        );
    }
    if let Some(client) = websearch_client {
        niles_tools::register_web_search_tool(&mut tools, client, websearch_default_num_results);
    }
    niles_tools::register_datetime_tool(&mut tools, &home.timezone);
    tools
}

/// Build a `NotificationCenter` from the `[notifications]` section.
fn build_notification_center(
    cfg: &niles_config::NotificationsConfig,
    timezone_str: &str,
) -> NotificationCenter {
    let mut center = NotificationCenter::new(cfg.capacity);
    if let Some(quiet) = cfg.to_quiet_hours_config(timezone_str) {
        center = center.with_quiet_hours(quiet);
    }
    center
}

/// Wyoming-backed delivery for the notification center.
struct WyomingDelivery {
    piper: Arc<niles_tts::PiperClient>,
    sender: niles_wyoming::WyomingSender,
    speakers: Arc<speakers::SpeakerRegistry>,
    satellites: Arc<SatelliteRegistry>,
    peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>>,
}

impl niles_notifications::NotificationDelivery for WyomingDelivery {
    fn deliver(
        &self,
        text: &str,
        room: Option<&str>,
        _priority: niles_notifications::Priority,
    ) -> bool {
        let Some(room_str) = room else {
            return false;
        };
        let Ok(room_name) = RoomName::parse(room_str) else {
            return false;
        };
        let peer = {
            let index = self.peer_index.lock().unwrap_or_else(|e| e.into_inner());
            index.get(&room_name).copied()
        };
        let Some(peer) = peer else {
            return false;
        };
        let piper = self.piper.clone();
        let sender = self.sender.clone();
        let speakers = self.speakers.clone();
        let satellites = self.satellites.clone();
        let text = text.to_string();
        tokio::spawn(async move {
            if let Err(e) =
                crate::speak::speak_back(&piper, &sender, peer, &text, &speakers, &satellites).await
            {
                tracing::warn!("[{peer}] notification speak-back failed: {e:#}");
            }
        });
        true
    }
}

/// Build a `MemoryStore` from the `[memory]` section.
/// Returns `None` when no directory is configured or when opening the store fails.
fn build_memory_store(cfg: &niles_config::MemoryConfig) -> Option<Arc<MemoryStore>> {
    let dir = cfg.directory.as_ref()?;
    match MemoryStore::open(niles_memory::MemoryConfig {
        directory: dir.clone(),
        user_char_limit: cfg.user_char_limit,
        agent_char_limit: cfg.agent_char_limit,
    }) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(
                "Failed to open memory store at {}: {e}; memory tools disabled",
                dir.display()
            );
            None
        }
    }
}

/// Build a `SkillStore` from the `[skills]` section.
/// Returns `None` when no directory is configured or when opening the store fails.
fn build_skill_store(cfg: &niles_config::SkillsConfig) -> Option<Arc<SkillStore>> {
    let dir = cfg.directory.as_ref()?;
    match SkillStore::open(dir, cfg.skill_max_chars, cfg.supporting_file_max_bytes) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(
                "Failed to open skill store at {}: {e}; skill tools disabled",
                dir.display()
            );
            None
        }
    }
}

/// Build an `OpenMeteoClient`.
/// Returns `None` when client construction fails.
fn build_weather_client() -> Option<Arc<niles_weather::OpenMeteoClient>> {
    niles_weather::OpenMeteoClient::new(niles_weather::OpenMeteoConfig::default())
        .inspect_err(|e| {
            tracing::warn!("Failed to build Open-Meteo client: {e}; weather tool disabled")
        })
        .ok()
        .map(Arc::new)
}

/// Build a `SearXngClient` from the `[web_search]` section.
/// Returns `None` when `base_url` is not configured or when client construction fails.
fn build_websearch_client(
    cfg: &niles_config::WebSearchConfig,
) -> Option<Arc<niles_websearch::SearXngClient>> {
    let base_url = cfg.base_url.as_ref()?;
    let config = niles_websearch::SearXngConfig {
        base_url: base_url.clone(),
        request_timeout: Duration::from_secs(cfg.timeout_seconds),
        user_agent: concat!(
            "niles/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/MarkNygaard/niles)"
        )
        .into(),
    };
    niles_websearch::SearXngClient::new(config)
        .inspect_err(|e| {
            tracing::warn!("Failed to build SearXNG client: {e}; web_search tool disabled")
        })
        .ok()
        .map(Arc::new)
}

fn spawn_skill_curator(
    store: Arc<SkillStore>,
    curator_cfg: niles_config::SkillsCuratorConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if !curator_cfg.enabled {
        tracing::info!("skill curator disabled by config");
        return None;
    }
    let interval = Duration::from_secs(curator_cfg.interval_hours * 3600);
    let thresholds = niles_skills::curator::CuratorThresholds {
        stale_after: chrono::Duration::days(curator_cfg.stale_after_days as i64),
        archive_after: chrono::Duration::days(curator_cfg.archive_after_days as i64),
    };
    Some(tokio::spawn(async move {
        // Defer the first sweep so startup work isn't competed with.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            match niles_skills::curator::apply_automatic_transitions(&store, Utc::now(), thresholds)
            {
                Ok(r) => tracing::info!(
                    examined = r.examined,
                    became_stale = r.became_stale,
                    became_archived = r.became_archived,
                    revived = r.revived,
                    skipped_pinned = r.skipped_pinned,
                    skipped_user_created = r.skipped_user_created,
                    "skill curator swept",
                ),
                Err(e) => tracing::warn!("skill curator sweep failed: {e:#}"),
            }
            tokio::time::sleep(interval).await;
        }
    }))
}

fn render_skills_section(out: &mut String, summaries: &[SkillSummary]) {
    out.push_str("\n\n# Available skills\n\n");
    let now = Utc::now();
    for s in summaries {
        let annotation = if s.status == SkillStatus::Stale {
            match s.last_activity_at {
                Some(last) => {
                    let days = (now - last).num_days().max(0);
                    format!(" [stale: {days} days unused]")
                }
                None => " [stale]".to_string(),
            }
        } else {
            String::new()
        };
        out.push_str(&format!(
            "- {} (v{}) — {}{}\n",
            s.name, s.version, s.description, annotation,
        ));
    }
}

/// Build a `PiperClient` from the `[tts]` section of an already-
/// validated config.
fn build_piper_client(cfg: &Config) -> anyhow::Result<PiperClient> {
    let piper_cfg = PiperConfig {
        base_url: cfg.tts.base_url.clone(),
        default_voice: cfg.tts.default_voice.clone(),
        request_timeout: Duration::from_secs(cfg.tts.timeout_seconds),
    };
    PiperClient::new(piper_cfg).context("building Piper HTTP client")
}

/// Build a `StateWriter` from the `[history]` section, pruning old
/// files on open. Returns a disabled no-op writer when no directory
/// is configured.
fn build_state_writer(cfg: &Config) -> anyhow::Result<Arc<StateWriter>> {
    let writer = match &cfg.history.directory {
        Some(dir) => {
            let w = StateWriter::new(dir).context("opening state history writer")?;
            if let Err(e) = w.prune(cfg.history.retention_days) {
                tracing::warn!("state history prune failed: {e:#}");
            }
            w
        }
        None => StateWriter::disabled(),
    };
    Ok(Arc::new(writer))
}

/// Build a `StateReader` from the `[history]` section. Returns a
/// disabled no-op reader when no directory is configured.
fn build_state_reader(cfg: &Config) -> Arc<StateReader> {
    match &cfg.history.directory {
        Some(dir) => Arc::new(StateReader::new(dir)),
        None => Arc::new(StateReader::disabled()),
    }
}

/// Append a device-state snapshot to the history log.
fn append_state_history(writer: &StateWriter, id: &DeviceId, state: &DeviceState) {
    if let Err(e) = writer.append(&StateEntry {
        ts: Utc::now(),
        device_id: id.clone(),
        state: state.clone(),
    }) {
        tracing::warn!("state history append failed: {e:#}");
    }
}

async fn transcribe(args: TranscribeArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let audio = std::fs::read(&args.audio)
        .with_context(|| format!("reading audio file {}", args.audio.display()))?;
    let filename = args
        .audio
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".into());

    let client = build_whisper_client(&cfg)?;

    eprintln!(
        "Transcribing {} ({} bytes) via {} ({}) ...",
        args.audio.display(),
        audio.len(),
        cfg.stt.base_url,
        cfg.stt.model
    );
    let transcript = client
        .transcribe(audio, &filename)
        .await
        .context("transcribing audio")?;

    if let Some(lang) = &transcript.language {
        eprintln!("Detected language: {lang}");
    }
    if let Some(dur) = transcript.duration_seconds {
        eprintln!("Audio duration: {dur:.2}s");
    }
    println!("{}", transcript.text);
    Ok(())
}

async fn synthesize(args: SynthesizeArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let client = build_piper_client(&cfg)?;
    let voice = args.voice.as_deref();
    let voice_for_log = voice.unwrap_or(&cfg.tts.default_voice);

    eprintln!(
        "Synthesizing {} chars via {} (voice {}) ...",
        args.text.chars().count(),
        cfg.tts.base_url,
        voice_for_log
    );
    let synthesis = client
        .synthesize(&args.text, voice)
        .await
        .context("synthesizing speech")?;

    std::fs::write(&args.out, &synthesis.audio_wav)
        .with_context(|| format!("writing WAV to {}", args.out.display()))?;
    println!(
        "Wrote {} bytes to {}",
        synthesis.audio_wav.len(),
        args.out.display()
    );
    Ok(())
}

const MAX_TOOL_ITERATIONS: usize = 5;

/// Tier A — stable system persona. Same on every LLM call so it
/// stays at the front of the system prompt where prompt caching
/// is most effective.
const NILES_SYSTEM_PERSONA: &str = "\
You are Niles, a home-automation assistant. You control lights, \
switches, scenes, and timers in a private home via the tools \
provided. Be concise and action-oriented: when the user asks you \
to do something, call the appropriate tool rather than describing \
what you would do. When you lack domain context for a request, \
you may have been given relevant capability references below; use \
them. If a needed capability isn't present, call \
look_up_capability to fetch one by name. Never invent device \
names — use the listing tools to discover what exists. You can \
save persistent skills using the mint_skill tool, but only when \
the user explicitly asks niles to remember or save a routine. \
Prefer patch_skill over mint_skill when an existing skill \
overlaps. Skill bodies should describe the how-to, not the \
conversation that produced them. When the Available skills \
section below lists a skill relevant to the request, call \
view_skill to read its full body.";

/// A minimal abstraction over the chat-completions endpoint so the
/// tool-calling loop is testable without spinning up an HTTP server.
/// `GroqClient` is the only real implementor; tests use a fake.
#[async_trait::async_trait]
pub(crate) trait ChatProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse>;
}

#[async_trait::async_trait]
impl<T: niles_llm::LlmBackend> ChatProvider for T {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(niles_llm::LlmBackend::chat(self, req).await?)
    }
}

/// Outcome of a tool-calling chat loop. The loop may complete with a
/// final text response, or abort early when the model requests
/// escalation to Tier 2.
#[derive(Debug)]
pub(crate) enum LoopOutcome {
    Done(String),
    EscalateRequested {
        reason: String,
        messages: Vec<Message>,
    },
}

/// Internal helper that starts from an existing message history.
async fn run_tool_calling_chat_with_messages(
    client: &dyn ChatProvider,
    registry: &ToolRegistry,
    mut messages: Vec<Message>,
    max_iterations: usize,
    exclude_tool: Option<&str>,
) -> anyhow::Result<(LoopOutcome, Vec<review::ToolTrace>)> {
    let mut llm_tools = registry.llm_tools();
    if let Some(name) = exclude_tool {
        llm_tools.retain(|t| t.name != name);
    }
    let mut trace: Vec<review::ToolTrace> = Vec::new();

    for _ in 0..max_iterations {
        let req = ChatRequest {
            messages: messages.clone(),
            tools: Some(llm_tools.clone()),
            tool_choice: Some(ToolChoice::Auto),
        };
        let resp = client.chat(req).await.context("calling LLM")?;

        if resp.tool_calls.is_empty() {
            return Ok((LoopOutcome::Done(resp.content.unwrap_or_default()), trace));
        }

        messages.push(Message::Assistant {
            content: None,
            tool_calls: Some(resp.tool_calls.clone()),
        });

        let escalate_reason = resp
            .tool_calls
            .iter()
            .find(|c| c.name == niles_tools::escalate::ESCALATE_TOOL_NAME)
            .and_then(|c| c.arguments.get("reason").and_then(|v| v.as_str()))
            .map(String::from);

        if let Some(reason) = escalate_reason {
            tracing::info!("Tier 1 requested escalation; skipping non-escalation tool execution");
            for call in &resp.tool_calls {
                let result = if call.name == niles_tools::escalate::ESCALATE_TOOL_NAME {
                    call.arguments.clone()
                } else {
                    json!({
                        "skipped": "deferred_to_tier2",
                        "reason": "tier1 requested escalation in this turn"
                    })
                };
                trace.push(review::ToolTrace {
                    tool: call.name.clone(),
                    arguments: call.arguments.clone(),
                    result: result.clone(),
                });
                messages.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content: result.to_string(),
                });
            }
            return Ok((LoopOutcome::EscalateRequested { reason, messages }, trace));
        }

        for call in &resp.tool_calls {
            let arg_keys: Vec<&str> = call
                .arguments
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            tracing::info!(
                "tool_call {name}({keys}) [id={id}]",
                id = call.id,
                name = call.name,
                keys = arg_keys.join(",")
            );

            let result = if call.name == niles_tools::escalate::ESCALATE_TOOL_NAME {
                call.arguments.clone()
            } else {
                match registry.execute(call).await {
                    Ok(v) => v,
                    Err(e) => json!({ "error": format!("{e}") }),
                }
            };
            tracing::info!(
                "tool_result {name} -> {body}",
                name = call.name,
                body = result.to_string()
            );
            trace.push(review::ToolTrace {
                tool: call.name.clone(),
                arguments: call.arguments.clone(),
                result: result.clone(),
            });
            messages.push(Message::Tool {
                tool_call_id: call.id.clone(),
                content: result.to_string(),
            });
        }
    }

    Err(anyhow::anyhow!(
        "chat tool-calling loop exhausted after {max_iterations} iterations without a final text answer"
    ))
}

/// Drive a chat conversation with tool calling until the model emits
/// a final text response or `max_iterations` is exhausted.
///
/// Each iteration sends the full message history to `client`. If the
/// response carries no tool calls, the function returns the assistant
/// content (or empty string if `content` is `None`). Otherwise the
/// tool calls are dispatched through `registry` and their results are
/// fed back as `Message::Tool` for the next iteration.
pub(crate) async fn run_tool_calling_chat(
    client: &dyn ChatProvider,
    registry: &ToolRegistry,
    prompt: &str,
    system_prompt: Option<&str>,
    max_iterations: usize,
) -> anyhow::Result<(LoopOutcome, Vec<review::ToolTrace>)> {
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(Message::System {
            content: sys.to_string(),
        });
    }
    messages.push(Message::User {
        content: prompt.to_string(),
    });
    run_tool_calling_chat_with_messages(client, registry, messages, max_iterations, None).await
}

async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = Arc::new(cfg.mqtt.z2m_prefix.clone());

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
        ambient_set,
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    tokio::time::sleep(Duration::from_secs(2)).await;

    let capability_loader = build_capability_loader(&cfg.capabilities);
    let capability_index = capability_loader.as_deref().map(build_capability_index);

    let memory_store = build_memory_store(&cfg.memory);
    let skill_store = build_skill_store(&cfg.skills);
    let weather_client = build_weather_client();
    let websearch_client = build_websearch_client(&cfg.web_search);
    let tools_registry = build_tool_registry(
        registry.clone(),
        publisher,
        z2m_prefix,
        capability_loader.clone(),
        memory_store.clone(),
        skill_store.clone(),
        weather_client.clone(),
        websearch_client.clone(),
        cfg.web_search.default_num_results,
        &cfg.home,
    );
    let client = build_groq_client(&cfg)?;
    eprintln!("Chatting via {} ({}) ...", cfg.llm.base_url, cfg.llm.model);

    let user_mem = memory_store
        .as_ref()
        .and_then(|s| match s.load(niles_memory::Target::User) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("failed to load user memory: {e}");
                None
            }
        });
    let agent_mem =
        memory_store
            .as_ref()
            .and_then(|s| match s.load(niles_memory::Target::Memory) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("failed to load agent memory: {e}");
                    None
                }
            });
    let user_mem_str = user_mem.as_deref().and_then(join_memory_entries);
    let agent_mem_str = agent_mem.as_deref().and_then(join_memory_entries);

    let skill_summaries = skill_store.as_ref().and_then(|s| match s.list_summaries() {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("failed to list skills: {e}");
            None
        }
    });

    let system_prompt = assemble_system_prompt_with_optional_capabilities(
        &args.prompt,
        &cfg.home,
        capability_index.as_ref(),
        capability_loader.as_deref(),
        None,
        user_mem_str.as_deref(),
        agent_mem_str.as_deref(),
        skill_summaries.as_deref(),
    );

    // Surface the actual error chain on failure. Both loop exhaustion
    // and a real LLM/network error end up here, but they're distinct
    // failure modes — printing `{e:#}` keeps the cause honest instead
    // of mislabeling every error as "loop exhausted".
    match run_tool_calling_chat(
        &client,
        &tools_registry,
        &args.prompt,
        Some(system_prompt.as_str()),
        MAX_TOOL_ITERATIONS,
    )
    .await
    {
        Ok((LoopOutcome::Done(text), _trace)) => println!("{text}"),
        Ok((LoopOutcome::EscalateRequested { reason, .. }, _trace)) => {
            eprintln!(
                "[niles chat] Tier 1 requested escalation ({reason}) but Tier 2 is not available in chat mode"
            );
        }
        Err(e) => eprintln!("[niles chat] {e:#}"),
    }

    source_handle.abort();
    Ok(())
}

async fn voice_tap(args: VoiceTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    let client = Arc::new(build_whisper_client(&cfg)?);
    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nTranscribing each utterance via {} ({}). Press Ctrl-C to exit.\n",
        cfg.stt.base_url, cfg.stt.model
    );

    let server_handle = tokio::spawn(server.run());
    let mut tracker = SessionTracker::new();

    loop {
        tokio::select! {
            // Drain queued events before reacting to a disconnect:
            // an `audio-stop` already in `rx` should complete its
            // session before `drop_peer` clears the in-flight slot.
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(session) = tracker.feed(incoming) {
                        // Hand transcription off to a task so a slow
                        // STT round-trip doesn't block the next
                        // satellite's audio from being buffered.
                        // Concurrency is unbounded by design for this
                        // dev tap — head-of-line blocking is worse
                        // than fan-out at small N. Prod dispatch will
                        // need a bounded worker pool.
                        let client = client.clone();
                        tokio::spawn(async move {
                            if let Some((peer, text)) = transcribe_session(&client, session).await {
                                println!("[{peer}] \"{text}\"");
                            }
                        });
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    // Free any half-buffered session for this peer
                    // immediately, rather than waiting on the 10-min
                    // idle reaper inside the server.
                    tracker.drop_peer(peer);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    // Detached transcription tasks (if any) are dropped here without
    // awaiting — fine for a dev tap; a graceful-shutdown signal for
    // those in-flight tasks is a separate concern.
    server_handle.abort();
    Ok(())
}

/// Wrap a session's PCM in WAV and ship it to Whisper. Returns the
/// (trimmed) transcript on success, `None` on any failure — errors
/// are logged in place so a single bad request doesn't take the
/// listener down.
async fn transcribe_session(
    client: &WhisperClient,
    session: niles_wyoming::AudioSession,
) -> Option<(SocketAddr, String)> {
    let pcm_format = PcmFormat {
        sample_rate_hz: session.format.sample_rate_hz,
        bits_per_sample: session.format.bits_per_sample,
        channels: session.format.channels,
    };
    let pcm_bytes = session.pcm.len();
    let wav = match pcm_to_wav(&session.pcm, pcm_format) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(
                "{}: WAV wrap failed ({pcm_bytes} PCM bytes): {e}",
                session.from
            );
            return None;
        }
    };

    match client.transcribe(wav, "session.wav").await {
        Ok(t) => Some((session.from, t.text.trim().to_string())),
        Err(e) => {
            tracing::warn!("{}: transcription failed: {e}", session.from);
            None
        }
    }
}

/// Spawn a fire-and-forget task that transcribes `session`, dispatches
/// the transcript, and speaks the response back to the satellite.
fn spawn_dispatch_task(
    whisper: &Arc<WhisperClient>,
    ctx: &DispatchCtx,
    piper: &Arc<PiperClient>,
    sender: &WyomingSender,
    session: niles_wyoming::AudioSession,
) {
    let whisper = whisper.clone();
    let ctx = ctx.clone();
    let piper = piper.clone();
    let sender = sender.clone();
    tokio::spawn(async move {
        if let Some((peer, text)) = transcribe_session(&whisper, session).await {
            let say = handle_transcript(&ctx, peer, &text).await;
            let entry = CommandEntry {
                ts: chrono::Utc::now(),
                peer,
                origin_room: ctx
                    .satellites
                    .room_for(peer)
                    .map(|r| r.as_str().to_string()),
                transcript: text.clone(),
                spoken_response: say.clone(),
            };
            if let Err(e) = ctx.history.append(&entry) {
                tracing::warn!("history append failed: {e:#}");
            }
            if let Some(say) = say {
                println!("[{peer}] say: {say}");
                if let Err(e) = crate::speak::speak_back(
                    &piper,
                    &sender,
                    peer,
                    &say,
                    &ctx.speakers,
                    &ctx.satellites,
                )
                .await
                {
                    tracing::warn!("[{peer}] speak-back failed: {e:#}");
                }
            }
        }
    });
}

async fn voice_dispatch(args: VoiceDispatchArgs) -> anyhow::Result<()> {
    // MQTT + config: connect_from_config validates and gives us a
    // live MqttClient. We grab a publisher handle *before* handing
    // the client to Z2mSource (which consumes it) so spawned
    // dispatch tasks can publish on the same connection.
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = Arc::new(cfg.mqtt.z2m_prefix.clone());

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;
    let whisper = Arc::new(build_whisper_client(&cfg)?);
    let piper = Arc::new(build_piper_client(&cfg)?);

    // Registry populated by Z2mSource. Dispatch tasks look up
    // devices in a room from this shared snapshot.
    let registry = Arc::new(DeviceRegistry::new());
    // Z2mSource requires a bus; we subscribe before spawning the
    // source so the device-index observer can't miss initial
    // DeviceAdded events while the registry warms up.
    let bus = EventBus::default();
    let mut bus_rx = bus.subscribe();
    let device_index = Arc::new(RwLock::new(build_initial_device_index(&registry)));
    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
        ambient_set,
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    // In-memory timer store, shared by the driver task (below) and
    // the dispatch context. The driver's behavior is documented on
    // `spawn_timer_driver` — see that helper for the 60 s sleep-cap
    // trade-off and the Pending → Ringing transition rules.
    let timers = Arc::new(TimerStore::new());
    let timer_handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

    // Build the LLM + tool registry once for the lifetime of the
    // server. Both go into DispatchCtx wrapped in Arc — they're cloned
    // (Arc::clone) into every spawned dispatch task. No Z2M warm-up
    // needed here: the first transcript arrives many seconds after
    // startup (wake-word + speech + STT round-trip), so Z2M has plenty
    // of time to populate before any tool call hits the registry.
    let llm = Arc::new(build_groq_client(&cfg)?);
    let capability_loader = build_capability_loader(&cfg.capabilities);
    let capability_index = capability_loader
        .as_deref()
        .map(build_capability_index)
        .map(Arc::new);

    let memory_store = build_memory_store(&cfg.memory);
    let skill_store = build_skill_store(&cfg.skills);
    let weather_client = build_weather_client();
    let websearch_client = build_websearch_client(&cfg.web_search);
    let mut tools = build_tool_registry(
        registry.clone(),
        publisher.clone(),
        z2m_prefix.clone(),
        capability_loader.clone(),
        memory_store.clone(),
        skill_store.clone(),
        weather_client.clone(),
        websearch_client.clone(),
        cfg.web_search.default_num_results,
        &cfg.home,
    );
    niles_tools::register_timer_tools(&mut tools, timers.clone());

    let command_writer = match &cfg.history.directory {
        Some(dir) => {
            let w = CommandWriter::new(dir).context("opening command history writer")?;
            if let Err(e) = w.prune(cfg.history.retention_days) {
                tracing::warn!("history prune failed: {e:#}");
            }
            w
        }
        None => CommandWriter::disabled(),
    };
    let command_writer = Arc::new(command_writer);
    let command_reader = match &cfg.history.directory {
        Some(dir) => Arc::new(CommandReader::new(dir)),
        None => Arc::new(CommandReader::disabled()),
    };
    niles_tools::register_history_tools(&mut tools, command_reader);

    let state_writer = build_state_writer(&cfg)?;
    let state_reader = build_state_reader(&cfg);
    niles_tools::register_state_history_tools(&mut tools, state_reader, registry.clone());

    let tier2 = build_tier2_client(&cfg)?;
    if tier2.is_some() {
        niles_tools::register_escalate_tool(&mut tools);
    }

    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    let satellites = Arc::new(SatelliteRegistry::from_config(&cfg.satellites));
    let speakers = Arc::new(speakers::SpeakerRegistry::from_config(&cfg.speakers));

    // Notification center
    let mut notifications = build_notification_center(&cfg.notifications, &cfg.home.timezone);
    let peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let wyoming_delivery = Arc::new(WyomingDelivery {
        piper: piper.clone(),
        sender: server.sender(),
        speakers: speakers.clone(),
        satellites: satellites.clone(),
        peer_index: peer_index.clone(),
    });
    notifications.set_delivery(wyoming_delivery);
    let notifications = Arc::new(notifications);
    niles_tools::register_notification_tools(&mut tools, notifications.clone());

    // Notification subscriber for timer expiry.
    let _timer_notification_handle = {
        let center = notifications.clone();
        let satellites = satellites.clone();
        let mut bus_rx = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match bus_rx.recv().await {
                    Ok(Event::TimerFired { name, origin, .. }) => {
                        let room = satellites.room_for(origin).map(|r| r.as_str().to_string());
                        let text = match name {
                            Some(n) => format!("'{n}' timer finished"),
                            None => "Timer finished".to_string(),
                        };
                        center.deliver(text, room, niles_notifications::Priority::Important);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("notification subscriber lagged by {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    let tools = Arc::new(tools);

    let mode_note = if args.dry_run {
        " (dry-run: nothing will be published)"
    } else {
        ""
    };
    eprintln!(
        "Wyoming server listening on tcp://{bind}\nZ2M source running on {prefix}/+/+; STT via {stt_url} ({model}).\nMatched intents trigger MQTT publish{mode}. Press Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix,
        stt_url = cfg.stt.base_url,
        model = cfg.stt.model,
        mode = mode_note,
    );

    let wyoming_sender = server.sender();
    let server_handle = tokio::spawn(server.run());
    let mut tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher,
        registry: registry.clone(),
        z2m_prefix,
        dry_run: args.dry_run,
        tracker: Arc::new(ManualModeTracker::new()),
        scenes: Arc::new(SceneStore::new()),
        timers,
        speakers,
        llm,
        tier2,
        tools,
        capability_loader,
        capability_index,
        satellites: satellites.clone(),
        device_index: Arc::clone(&device_index),
        history: command_writer,
        memory: memory_store.unwrap_or_else(|| Arc::new(MemoryStore::disabled())),
        skill_store,
        home: Arc::new(cfg.home.clone()),
        review: cfg.skills.review.clone(),
    };

    // Keep the device index in sync so Tier-0 device-name matchers
    // work in voice-dispatch mode too (same pattern as `serve`).
    let observer_device_index = Arc::clone(&ctx.device_index);
    let observer_state_writer = state_writer.clone();
    let _device_index_handle = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(Event::DeviceAdded { device }) if device.is_light() => {
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(device.id.clone());
                }
                Ok(Event::DeviceRemoved { id }) => {
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                }
                Ok(Event::DeviceStateChanged { id, state }) => {
                    append_state_history(&observer_state_writer, &id, &state);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                _ => {}
            }
        }
    });

    loop {
        tokio::select! {
            // See voice-tap: drain events before disconnects so a
            // trailing `audio-stop` isn't dropped by the race.
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(room) = satellites.room_for(incoming.from) {
                        peer_index.lock().unwrap_or_else(|e| e.into_inner()).insert(room.clone(), incoming.from);
                    }
                    if let Some(session) = tracker.feed(incoming) {
                        // Same unbounded fan-out as voice-tap — fine
                        // for a dev tool, replaced by a bounded
                        // worker pool when this becomes prod dispatch.
                        spawn_dispatch_task(&whisper, &ctx, &piper, &wyoming_sender, session);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    tracker.drop_peer(peer);
                    let mut index = peer_index.lock().unwrap_or_else(|e| e.into_inner());
                    index.retain(|_, addr| *addr != peer);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    server_handle.abort();
    source_handle.abort();
    timer_handle.abort();
    Ok(())
}

/// Shared by every spawned dispatch task. Cheaply cloneable —
/// `MqttPublisher`, `Arc`, `bool` all clone in constant time.
#[derive(Clone)]
struct DispatchCtx {
    publisher: MqttPublisher,
    registry: Arc<DeviceRegistry>,
    z2m_prefix: Arc<String>,
    dry_run: bool,
    tracker: Arc<ManualModeTracker>,
    scenes: Arc<SceneStore>,
    timers: Arc<TimerStore>,
    speakers: Arc<speakers::SpeakerRegistry>,
    llm: Arc<GroqClient>,
    tier2: Option<Arc<dyn ChatProvider>>,
    tools: Arc<ToolRegistry>,
    capability_loader: Option<Arc<CapabilityLoader>>,
    capability_index: Option<Arc<niles_intent::CapabilityIndex>>,
    satellites: Arc<SatelliteRegistry>,
    device_index: Arc<RwLock<DeviceIndex>>,
    history: Arc<CommandWriter>,
    memory: Arc<MemoryStore>,
    skill_store: Option<Arc<SkillStore>>,
    home: Arc<niles_config::HomeConfig>,
    review: niles_config::SkillsReviewConfig,
}

/// Parse a transcript and act on any Tier 0 intent it produces.
async fn handle_transcript(ctx: &DispatchCtx, peer: SocketAddr, text: &str) -> Option<String> {
    // `transcribe_session` already trims, so an empty `text` here means
    // Whisper returned nothing for a silent/noise session. Don't burn
    // a Groq round-trip on it — Tier 0 wouldn't match either.
    if text.is_empty() {
        tracing::debug!("[{peer}] empty transcript, skipping dispatch");
        return None;
    }

    let origin_room = ctx.satellites.room_for(peer);

    // IntentRouter is a zero-sized unit struct; the regexes are
    // compiled once into a static OnceLock, so constructing one
    // per call is free.
    let parsed = {
        let idx = ctx.device_index.read().unwrap_or_else(|e| e.into_inner());
        let router_ctx = RouterContext {
            device_index: &idx,
            origin_room,
        };
        IntentRouter::new().parse_with_context(text, router_ctx)
    };

    let intent = match parsed {
        Some(i) => i,
        None => {
            // Tier 0 miss — escalate to Tier 1 LLM with the tool registry.
            tracing::info!("[{peer}] Tier 0 miss, escalating to LLM: {text:?}");
            let user_mem = match ctx.memory.load(niles_memory::Target::User) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("[{peer}] failed to load user memory: {e}");
                    None
                }
            };
            let agent_mem = match ctx.memory.load(niles_memory::Target::Memory) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("[{peer}] failed to load agent memory: {e}");
                    None
                }
            };
            let user_mem_str = user_mem.as_deref().and_then(join_memory_entries);
            let agent_mem_str = agent_mem.as_deref().and_then(join_memory_entries);
            let skill_summaries = ctx
                .skill_store
                .as_ref()
                .and_then(|s| match s.list_summaries() {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!("[{peer}] failed to list skills: {e}");
                        None
                    }
                });
            let system_prompt = assemble_system_prompt_with_optional_capabilities(
                text,
                &ctx.home,
                ctx.capability_index.as_deref(),
                ctx.capability_loader.as_deref(),
                origin_room,
                user_mem_str.as_deref(),
                agent_mem_str.as_deref(),
                skill_summaries.as_deref(),
            );
            match run_tool_calling_chat(
                ctx.llm.as_ref(),
                ctx.tools.as_ref(),
                text,
                Some(system_prompt.as_str()),
                MAX_TOOL_ITERATIONS,
            )
            .await
            {
                Ok((LoopOutcome::Done(response), tool_trace)) => {
                    println!("[{peer}] \"{text}\" -> (Tier 1) {response}");
                    let memory_for_review = ctx.memory.is_enabled().then(|| ctx.memory.clone());
                    let snapshot = review::ReviewSnapshot {
                        transcript: text.to_string(),
                        spoken_response: response.clone(),
                        tool_trace,
                        user_memory: user_mem_str.clone(),
                        agent_memory: agent_mem_str.clone(),
                        skill_summaries: skill_summaries.clone().unwrap_or_default(),
                    };
                    if ctx.review.enabled
                        && (memory_for_review.is_some() || ctx.skill_store.is_some())
                        && review::is_reviewable_turn(&snapshot)
                    {
                        let _handle = review::spawn_skill_review(
                            snapshot,
                            memory_for_review,
                            ctx.skill_store.clone(),
                            ctx.llm.clone(),
                            ctx.home.clone(),
                            ctx.review.max_iters as usize,
                        );
                    }
                    return Some(response);
                }
                Ok((LoopOutcome::EscalateRequested { reason, messages }, _tool_trace)) => {
                    tracing::info!("[{peer}] Tier 1 requested escalation: {reason}");
                    match &ctx.tier2 {
                        Some(tier2) => {
                            println!("[{peer}] \"{text}\" -> escalating to Tier 2 ({reason})");
                            match run_tool_calling_chat_with_messages(
                                tier2.as_ref(),
                                ctx.tools.as_ref(),
                                messages,
                                MAX_TOOL_ITERATIONS,
                                Some(niles_tools::escalate::ESCALATE_TOOL_NAME),
                            )
                            .await
                            {
                                Ok((LoopOutcome::Done(response), _)) => {
                                    println!("[{peer}] \"{text}\" -> (Tier 2) {response}");
                                    return Some(response);
                                }
                                Ok((LoopOutcome::EscalateRequested { .. }, _)) => {
                                    println!(
                                        "[{peer}] \"{text}\" -> (Tier 2) escalation requested; ignoring"
                                    );
                                    tracing::warn!(
                                        "[{peer}] Tier 2 also requested escalation; ignoring"
                                    );
                                    return Some(
                                        "Sorry, I'm not able to handle that request.".into(),
                                    );
                                }
                                Err(e) => {
                                    println!("[{peer}] \"{text}\" -> (Tier 2) error: {e:#}");
                                    tracing::warn!("[{peer}] Tier 2 dispatch failed: {e:#}");
                                    return Some("Sorry, something went wrong.".into());
                                }
                            }
                        }
                        None => {
                            println!(
                                "[{peer}] \"{text}\" -> (Tier 1) escalation requested but Tier 2 not configured"
                            );
                            tracing::warn!("[{peer}] Tier 2 not configured, falling through");
                            return Some(
                                "Sorry, I'm not able to escalate that request right now.".into(),
                            );
                        }
                    }
                }
                Err(e) => {
                    // Mirror the success-path stdout line so a Tier 1
                    // failure is visible without enabling tracing.
                    println!("[{peer}] \"{text}\" -> (Tier 1) error: {e:#}");
                    tracing::warn!("[{peer}] Tier 1 LLM dispatch failed: {e:#}");
                    return Some("Sorry, something went wrong.".into());
                }
            }
        }
    };

    println!("[{peer}] \"{text}\" -> {}", format_intent(&intent));
    match intent {
        Intent::LightSet { room, on } => {
            let (_canonical, targets) =
                match resolve_room_targets(ctx, peer, &room, |d| d.state.on.is_some()) {
                    RoomResolve::Found(c, t) => (c, t),
                    RoomResolve::BadName => return Some(response::room_not_found(&room)),
                    RoomResolve::NoDevices => return Some(response::room_no_devices(&room)),
                    RoomResolve::WarmingUp => return Some(response::room_warming_up()),
                };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_set(&room, on))
        }
        Intent::LightSetAll { on } => {
            let targets: Vec<Device> = ctx
                .registry
                .list_all()
                .into_iter()
                .filter(|d| d.is_light())
                .collect();
            if targets.is_empty() {
                if ctx.registry.is_empty() {
                    println!(
                        "[{peer}] registry is still warming up (no devices yet) — try again in a moment"
                    );
                    return Some(response::room_warming_up());
                }
                println!("[{peer}] no lights in registry — nothing to dispatch");
                return Some(response::no_lights());
            }
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::all_lights(on))
        }
        Intent::LightDim { room, percent } => {
            let (_canonical, targets) =
                match resolve_room_targets(ctx, peer, &room, |d| d.state.brightness.is_some()) {
                    RoomResolve::Found(c, t) => (c, t),
                    RoomResolve::BadName => return Some(response::room_not_found(&room)),
                    RoomResolve::NoDevices => return Some(response::room_no_devices(&room)),
                    RoomResolve::WarmingUp => return Some(response::room_warming_up()),
                };
            // Flag each dimmable target *before* publishing so the
            // curve driver can't race a tick in between and overwrite
            // the dim we're about to send.
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                brightness: Some(percent),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_dim(&room, percent))
        }
        Intent::LightStep {
            room,
            delta_percent,
        } => {
            let (_canonical, targets) =
                match resolve_room_targets(ctx, peer, &room, |d| d.state.brightness.is_some()) {
                    RoomResolve::Found(c, t) => (c, t),
                    RoomResolve::BadName => return Some(response::room_not_found(&room)),
                    RoomResolve::NoDevices => return Some(response::room_no_devices(&room)),
                    RoomResolve::WarmingUp => return Some(response::room_warming_up()),
                };
            // `resolve_room_targets` already filtered for devices with
            // brightness, so every target has a known value.
            let base = (targets
                .iter()
                .map(|d| d.state.brightness.unwrap() as i32)
                .sum::<i32>()
                / targets.len() as i32) as i16;
            let new = (base + delta_percent).clamp(0, 100) as u8;
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                brightness: Some(new),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_dim(&room, new))
        }
        Intent::LightKelvinStep { room, delta_kelvin } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.is_curve_driven() && d.supports_color_temperature()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName => return Some(response::room_not_found(&room)),
                RoomResolve::NoDevices => {
                    return Some(response::room_no_devices(&room));
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            // `resolve_room_targets` already filtered for devices with
            // color temperature, so every target has a known value.
            let base = (targets
                .iter()
                .map(|d| d.state.color_temp_kelvin.unwrap() as u32)
                .sum::<u32>()
                / targets.len() as u32) as u16;
            let new = base.saturating_add_signed(delta_kelvin).clamp(2000, 6500);
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                color_temp_kelvin: Some(new),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_kelvin_step(&room, new))
        }
        Intent::LightKelvinSet { room, kelvin } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.is_curve_driven() && d.supports_color_temperature()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName => return Some(response::room_not_found(&room)),
                RoomResolve::NoDevices => {
                    return Some(response::room_no_devices(&room));
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            let target_kelvin = kelvin.clamp(2000, 6500);
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                color_temp_kelvin: Some(target_kelvin),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_kelvin_set(&room, target_kelvin))
        }
        Intent::DeviceSet { device_id, on } => {
            let Some(device) = ctx.registry.get(&device_id) else {
                return Some(response::device_not_found(&device_id));
            };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            publish_single(ctx, peer, &device, &desired).await;
            ctx.tracker.flag(&device.id);
            Some(response::device_set(&device_id, on))
        }
        Intent::DeviceDim { device_id, percent } => {
            let Some(device) = ctx.registry.get(&device_id) else {
                return Some(response::device_not_found(&device_id));
            };
            // Flag before publishing so the curve driver can't race
            // a tick in between and overwrite the dim we're about to send.
            ctx.tracker.flag(&device.id);
            let desired = DeviceState {
                brightness: Some(percent),
                ..Default::default()
            };
            publish_single(ctx, peer, &device, &desired).await;
            Some(response::device_dim(&device_id, percent))
        }
        Intent::SceneSave { name, room } => {
            let canonical = match room.as_deref().map(intent_room_to_canonical) {
                Some(Ok(r)) => Some(r),
                Some(Err(reason)) => {
                    let room_name = room.as_deref().unwrap_or("");
                    tracing::warn!(
                        "[{peer}] room {room:?} is not a valid registry name: {reason}",
                        room = room_name,
                    );
                    return Some(response::room_not_found(room_name));
                }
                None => None,
            };
            let n = ctx.scenes.save(&name, &ctx.registry, canonical.as_ref());
            match &canonical {
                Some(r) => println!("[{peer}] saved scene {name:?} with {n} devices in {r}"),
                None => println!("[{peer}] saved scene {name:?} with {n} devices (whole home)"),
            }
            Some(response::scene_saved(&name))
        }
        Intent::SceneApply { name } => {
            let Some(entries) = ctx.scenes.get(&name) else {
                println!("[{peer}] no scene named {name:?}");
                return Some(response::scene_not_found(&name));
            };
            if entries.is_empty() {
                println!("[{peer}] scene {name:?} is empty — nothing to apply");
                return Some(response::scene_empty(&name));
            }
            for entry in entries {
                let (topic, payload) =
                    format_set_command(&ctx.z2m_prefix, &entry.device_id, &entry.state);
                if ctx.dry_run {
                    println!("[{peer}] [dry-run] {topic}  {payload}");
                } else {
                    match ctx.publisher.publish(&topic, payload.clone()).await {
                        Ok(()) => println!("[{peer}] published {topic}  {payload}"),
                        Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
                    }
                }
                // ARCHITECTURE.md:501 — scene-applied lights enter
                // manual mode until the user explicitly clears them.
                ctx.tracker.flag(&entry.device_id);
            }
            println!("[{peer}] applied scene {name:?}");
            Some(response::scene_applied(&name))
        }
        Intent::SceneList => {
            let names = ctx.scenes.names();
            if names.is_empty() {
                println!("[{peer}] no scenes saved yet");
            } else {
                println!("[{peer}] {} scenes: {}", names.len(), names.join(", "));
            }
            Some(response::scene_list(&names))
        }
        Intent::SceneDelete { name } => {
            if ctx.scenes.delete(&name) {
                println!("[{peer}] deleted scene {name:?}");
                Some(response::scene_deleted(&name))
            } else {
                println!("[{peer}] no scene named {name:?}");
                Some(response::scene_not_found(&name))
            }
        }
        Intent::ClearManualMode { room } => match room {
            None => {
                let n = ctx.tracker.clear_all();
                println!("[{peer}] back to normal -> cleared manual flag on {n} devices");
                Some(response::cleared_manual(None))
            }
            Some(name) => {
                let canonical = match intent_room_to_canonical(&name) {
                    Ok(r) => r,
                    Err(reason) => {
                        tracing::warn!(
                            "[{peer}] room {name:?} is not a valid registry name: {reason}"
                        );
                        return Some(response::room_not_found(&name));
                    }
                };
                let n = ctx.tracker.clear_room(&canonical);
                println!(
                    "[{peer}] back to normal in {name} -> cleared manual flag on {n} devices in {canonical}"
                );
                Some(response::cleared_manual(Some(&name)))
            }
        },
        Intent::MediaPause { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(ctx, &room, |c| async move { c.pause().await }).await,
            response::media_pause(&room),
            "pause",
        ),
        Intent::MediaPlay { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(ctx, &room, |c| async move { c.play().await }).await,
            response::media_play(&room),
            "play",
        ),
        Intent::MediaNext { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(ctx, &room, |c| async move { c.next().await }).await,
            response::media_next(&room),
            "next",
        ),
        Intent::MediaPrevious { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(ctx, &room, |c| async move { c.previous().await }).await,
            response::media_previous(&room),
            "previous",
        ),
        Intent::MediaVolumeSet { room, percent } => media_result_to_response(
            peer,
            &room,
            media_dispatch(
                ctx,
                &room,
                move |c| async move { c.set_volume(percent).await },
            )
            .await,
            response::media_volume(&room, percent),
            "volume set",
        ),
        Intent::MediaVolumeStep { room, delta } => {
            let canonical = match intent_room_to_canonical(&room) {
                Ok(r) => r,
                Err(reason) => {
                    tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
                    return Some(response::room_not_found(&room));
                }
            };
            let client = match ctx.speakers.get(&canonical) {
                Some(c) => c,
                None => {
                    println!("[{peer}] no speaker in {canonical}");
                    return Some(response::no_speaker_in_room(canonical.as_str()));
                }
            };
            let current = match client.get_volume().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("[{peer}] speaker unreachable in {canonical}: {e}");
                    return Some(response::speaker_unreachable(canonical.as_str()));
                }
            };
            let new = (current as i16 + delta).clamp(0, 100) as u8;
            if let Err(e) = client.set_volume(new).await {
                tracing::warn!("[{peer}] speaker unreachable in {canonical}: {e}");
                return Some(response::speaker_unreachable(canonical.as_str()));
            }
            println!("[{peer}] media volume step in {canonical}: {current} -> {new}");
            Some(response::media_volume(canonical.as_str(), new))
        }
        Intent::TimerSet { duration, name } => {
            let id = ctx.timers.set(duration, name.clone(), peer, Utc::now());
            let label = timer_label_for(name.as_deref(), duration);
            println!("[{peer}] {label} started (id={})", id.0);
            Some(response::timer_started(duration, name.as_deref()))
        }
        Intent::Stop | Intent::Cancel => {
            let stopped_entry = ctx.timers.stop_most_recent_ringing();
            if let Some(entry) = &stopped_entry {
                println!("[{peer}] stopped {}", timer_label(entry));
            } else {
                println!("[{peer}] nothing ringing to stop");
            }
            Some(response::stopped(stopped_entry.is_some()))
        }
        Intent::TimerCancel { name } => {
            let n = ctx.timers.cancel_by_name(&name);
            if n == 0 {
                println!("[{peer}] no timer named {name:?}");
            } else {
                println!("[{peer}] cancelled {n} timer(s) named {name:?}");
            }
            Some(response::timer_cancelled(&name, n))
        }
        Intent::TimerList => {
            let entries = ctx.timers.list();
            if entries.is_empty() {
                println!("[{peer}] no active timers");
            } else {
                println!("[{peer}] {} active timer(s):", entries.len());
                for e in &entries {
                    println!(
                        "  - id={} {} (state={:?}, expires_at={})",
                        e.id.0,
                        timer_label(e),
                        e.state,
                        e.expires_at,
                    );
                }
            }
            Some(response::timer_list(entries.len()))
        }
        _ => {
            tracing::info!("{peer}: unknown intent variant, skipping dispatch");
            Some(response::fallback())
        }
    }
}

/// Outcome of resolving a room name to a device list.
enum RoomResolve {
    Found(RoomName, Vec<Device>),
    BadName,
    NoDevices,
    WarmingUp,
}

/// Resolve a transcript-derived room reference into the canonical
/// `RoomName` + the list of devices in that room that pass
/// `capability_filter`.
///
/// Centralizing this avoids the previous duplicated lookup in the
/// `LightDim` arm (one walk to flag, another inside `dispatch_room`
/// to publish).
fn resolve_room_targets<F>(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    room: &str,
    capability_filter: F,
) -> RoomResolve
where
    F: Fn(&niles_core::Device) -> bool,
{
    let canonical = match intent_room_to_canonical(room) {
        Ok(r) => r,
        Err(reason) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            return RoomResolve::BadName;
        }
    };

    let targets: Vec<_> = ctx
        .registry
        .list_room(&canonical)
        .into_iter()
        .filter(&capability_filter)
        .collect();

    if targets.is_empty() {
        // Distinguish startup race from a genuinely empty room.
        // The registry is populated asynchronously from Z2M's
        // retained `bridge/devices`; on a cold start it may not
        // have arrived yet by the time the first utterance lands.
        if ctx.registry.is_empty() {
            println!(
                "[{peer}] registry is still warming up (no devices yet) — try again in a moment"
            );
            RoomResolve::WarmingUp
        } else {
            println!(
                "[{peer}] no devices in room '{canonical}' support this action — nothing to dispatch"
            );
            RoomResolve::NoDevices
        }
    } else {
        RoomResolve::Found(canonical, targets)
    }
}

/// Publish the requested target state to each device in `targets`.
/// Pure dispatch — room resolution + capability filtering happened
/// upstream in [`resolve_room_targets`].
async fn dispatch_to_targets(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    targets: &[Device],
    desired: &DeviceState,
) {
    debug_assert!(
        is_actionable(desired),
        "dispatch_to_targets called with a non-actionable target state"
    );

    for device in targets {
        let (topic, payload) = format_set_command(&ctx.z2m_prefix, &device.id, desired);
        if ctx.dry_run {
            println!("[{peer}] [dry-run] {topic}  {payload}");
            continue;
        }
        match ctx.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => println!("[{peer}] published {topic}  {payload}"),
            Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
        }
    }
}

/// Publish a state update to a single device.
///
/// The caller is responsible for `tracker.flag()` timing: flag
/// *before* publish for dim operations so the curve driver can't
/// race, and *after* publish for on/off (same pattern as
/// `dispatch_to_targets`).
async fn publish_single(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    device: &Device,
    desired: &DeviceState,
) {
    let (topic, payload) = format_set_command(&ctx.z2m_prefix, &device.id, desired);
    if ctx.dry_run {
        println!("[{peer}] [dry-run] {topic}  {payload}");
    } else {
        match ctx.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => println!("[{peer}] published {topic}  {payload}"),
            Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
        }
    }
}

/// Convert a transcript-style room reference ("living room") into a
/// registry [`RoomName`] ("living_room"). This is the single
/// normalization point between intent output and registry lookup:
/// trims, lowercases, swaps whitespace for underscores, then
/// validates via `RoomName::parse`.
fn intent_room_to_canonical(s: &str) -> std::result::Result<RoomName, String> {
    let normalized = s.trim().to_ascii_lowercase().replace([' ', '\t'], "_");
    RoomName::parse(&normalized).map_err(|e| format!("{e}"))
}

#[derive(Debug)]
enum MediaDispatchError {
    BadRoom(String),
    NoSpeaker(RoomName),
    Unreachable(RoomName, String),
}

async fn media_dispatch<F, Fut>(
    ctx: &DispatchCtx,
    room_raw: &str,
    op: F,
) -> std::result::Result<(), MediaDispatchError>
where
    F: FnOnce(Arc<SonosClient>) -> Fut,
    Fut: std::future::Future<Output = niles_speakers::Result<()>>,
{
    let canonical = intent_room_to_canonical(room_raw).map_err(MediaDispatchError::BadRoom)?;
    let client = ctx
        .speakers
        .get(&canonical)
        .ok_or_else(|| MediaDispatchError::NoSpeaker(canonical.clone()))?;
    op(client)
        .await
        .map_err(|e| MediaDispatchError::Unreachable(canonical, e.to_string()))
}

/// Turns the result of a `media_dispatch` call into the spoken
/// response the user hears, logging warnings for error cases.
fn media_result_to_response(
    peer: SocketAddr,
    room: &str,
    result: std::result::Result<(), MediaDispatchError>,
    ok_msg: String,
    action: &str,
) -> Option<String> {
    match result {
        Ok(()) => {
            println!("[{peer}] media {action} in {room}");
            Some(ok_msg)
        }
        Err(MediaDispatchError::BadRoom(reason)) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            Some(response::room_not_found(room))
        }
        Err(MediaDispatchError::NoSpeaker(r)) => {
            println!("[{peer}] no speaker in {r}");
            Some(response::no_speaker_in_room(r.as_str()))
        }
        Err(MediaDispatchError::Unreachable(r, e)) => {
            tracing::warn!("[{peer}] speaker unreachable in {r}: {e}");
            Some(response::speaker_unreachable(r.as_str()))
        }
    }
}

/// Current wall-clock time in `tz` converted to a [`MinuteOfDay`].
/// Returns `None` and logs an error if the conversion fails (should
/// be impossible with chrono's contract). The returned `DateTime`
/// is the same instant used for the conversion.
fn current_minute_of_day(
    tz: chrono_tz::Tz,
) -> Option<(MinuteOfDay, chrono::DateTime<chrono_tz::Tz>)> {
    let now = Utc::now().with_timezone(&tz);
    let hour = u8::try_from(now.hour()).expect("chrono::Timelike::hour is 0..=23");
    let minute = u8::try_from(now.minute()).expect("chrono::Timelike::minute is 0..=59");
    match MinuteOfDay::new(hour, minute) {
        Ok(m) => Some((m, now)),
        Err(e) => {
            tracing::error!("could not construct MinuteOfDay from {now}: {e}");
            None
        }
    }
}

fn load_timer_store(dir: Option<&Path>) -> TimerStore {
    let Some(dir) = dir else {
        return TimerStore::new();
    };
    let path = dir.join("timers.json");
    match TimerStore::load_from_file(&path) {
        Ok(s) => s.with_persistence(path),
        Err(e) => {
            tracing::warn!("persistence: timers load failed ({e}); starting empty");
            TimerStore::new().with_persistence(path)
        }
    }
}

fn load_scene_store(dir: Option<&Path>) -> SceneStore {
    let Some(dir) = dir else {
        return SceneStore::new();
    };
    let path = dir.join("scenes.json");
    match SceneStore::load_from_file(&path) {
        Ok(s) => s.with_persistence(path),
        Err(e) => {
            tracing::warn!("persistence: scenes load failed ({e}); starting empty");
            SceneStore::new().with_persistence(path)
        }
    }
}

fn load_morning_claims(dir: Option<&Path>) -> MorningClaimTracker {
    let Some(dir) = dir else {
        return MorningClaimTracker::new();
    };
    let path = dir.join("morning_claims.json");
    match MorningClaimTracker::load_from_file(&path) {
        Ok(s) => s.with_persistence(path),
        Err(e) => {
            tracing::warn!("persistence: morning_claims load failed ({e}); starting empty");
            MorningClaimTracker::new().with_persistence(path)
        }
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = Arc::new(cfg.mqtt.z2m_prefix.clone());

    let api_bind = cfg
        .api
        .socket_addr()
        .context("resolving api.bind_address")?;
    let wyoming_bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    let tz: chrono_tz::Tz = cfg.home.timezone.parse().map_err(|e| {
        anyhow::anyhow!(
            "home.timezone '{}' is not a valid IANA zone: {e}",
            cfg.home.timezone
        )
    })?;
    let curve = cfg
        .lighting
        .to_curve_config()
        .context("converting [lighting] section to a CurveConfig")?;
    let morning_routine = cfg
        .lighting
        .morning_routine
        .as_ref()
        .map(|dto| {
            dto.to_morning_routine_config()
                .context("converting [lighting.morning_routine] to MorningRoutineConfig")
        })
        .transpose()?;

    let whisper = Arc::new(build_whisper_client(&cfg)?);
    let piper = Arc::new(build_piper_client(&cfg)?);

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let tracker = Arc::new(ManualModeTracker::new());
    let claim_tracker = Arc::new(load_morning_claims(cfg.persistence.directory.as_deref()));

    // Device index: built from the registry at startup and updated
    // on DeviceAdded / DeviceRemoved so Tier 0 matchers can resolve
    // spoken device names without an LLM round-trip.
    let device_index = Arc::new(RwLock::new(build_initial_device_index(&registry)));

    // Subscribe to the bus *before* spawning the source so we can't miss
    // the early DeviceStateChanged events that seed the observer's
    // last-seen on/off map — broadcast channels only deliver messages
    // sent after a receiver is bound.
    let state_writer = build_state_writer(&cfg)?;
    let observer_tracker = tracker.clone();
    let observer_claim_tracker = claim_tracker.clone();
    let observer_registry = registry.clone();
    let observer_device_index = Arc::clone(&device_index);
    let observer_publisher = publisher.clone();
    let observer_z2m_prefix = z2m_prefix.clone();
    let observer_dry_run = args.dry_run;
    let observer_state_writer = state_writer.clone();
    let mut bus_rx = bus.subscribe();

    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
        ambient_set,
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    // EventBus observer — feeds tracker.observe() for off→on
    // auto-clear, and tracker.forget() so removed devices don't
    // leave stale tracker entries behind (the maps would otherwise
    // grow monotonically over a long-running service).
    // Also releases morning-routine claims when a device is turned
    // off mid-ramp, and forgets claims on device removal.
    let observer_handle = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(Event::DeviceStateChanged { id, state }) => {
                    observer_tracker.observe(&id, &state);
                    // Mid-ramp off cancels the routine for the rest of today.
                    if state.on == Some(false) && observer_claim_tracker.is_claimed(&id) {
                        observer_claim_tracker.release(&id);
                    }
                    append_state_history(&observer_state_writer, &id, &state);
                }
                Ok(Event::DeviceAdded { device }) => {
                    if device.is_light() {
                        observer_device_index
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(device.id.clone());
                    }
                }
                Ok(Event::DeviceRemoved { id }) => {
                    observer_tracker.forget(&id);
                    observer_claim_tracker.forget(&id);
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                }
                Ok(Event::DeviceAction { id, action }) => {
                    let Some(effect) = classify_action(&action) else {
                        // Includes _release variants and unknown strings.
                        continue;
                    };
                    let room = id.room().clone();
                    let targets: Vec<Device> = observer_registry
                        .list_room(&room)
                        .into_iter()
                        .filter(|d| d.is_light() && d.id != id)
                        .collect();
                    if targets.is_empty() {
                        tracing::debug!("switch {id} pressed but no actionable lights in {room}");
                        continue;
                    }
                    let desired = match effect {
                        SwitchEffect::TurnOnRoom => DeviceState {
                            on: Some(true),
                            ..Default::default()
                        },
                        SwitchEffect::TurnOffRoom => DeviceState {
                            on: Some(false),
                            ..Default::default()
                        },
                        SwitchEffect::StepBrightness { delta_percent } => {
                            let known: Vec<u8> =
                                targets.iter().filter_map(|d| d.state.brightness).collect();
                            let base: i16 = if known.is_empty() {
                                50
                            } else {
                                (known.iter().copied().map(|b| b as i32).sum::<i32>()
                                    / known.len() as i32) as i16
                            };
                            let next = (base + delta_percent).clamp(0, 100) as u8;
                            DeviceState {
                                brightness: Some(next),
                                ..Default::default()
                            }
                        }
                    };
                    for d in &targets {
                        observer_tracker.flag(&d.id);
                    }
                    for d in &targets {
                        let (topic, payload) =
                            format_set_command(&observer_z2m_prefix, &d.id, &desired);
                        if observer_dry_run {
                            println!("[switch] [dry-run] {topic}  {payload}");
                        } else {
                            match observer_publisher.publish(&topic, payload.clone()).await {
                                Ok(()) => println!("[switch] published {topic}  {payload}"),
                                Err(e) => tracing::warn!("[switch] publish to {topic} failed: {e}"),
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("ManualModeTracker observer lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // No Z2M warm-up needed here: tool calls only fire after a
    // transcript arrives, by which point Z2M has had seconds to
    // populate. See voice_dispatch for the same reasoning.
    let llm = Arc::new(build_groq_client(&cfg)?);
    let timers = Arc::new(load_timer_store(cfg.persistence.directory.as_deref()));
    let scenes = Arc::new(load_scene_store(cfg.persistence.directory.as_deref()));
    let capability_loader = build_capability_loader(&cfg.capabilities);
    let capability_index = capability_loader
        .as_deref()
        .map(build_capability_index)
        .map(Arc::new);

    let memory_store = build_memory_store(&cfg.memory);
    let skill_store = build_skill_store(&cfg.skills);
    let weather_client = build_weather_client();
    let _skill_curator_handle = skill_store
        .clone()
        .and_then(|store| spawn_skill_curator(store, cfg.skills.curator.clone()));
    let websearch_client = build_websearch_client(&cfg.web_search);
    let mut tools = build_tool_registry(
        registry.clone(),
        publisher.clone(),
        z2m_prefix.clone(),
        capability_loader.clone(),
        memory_store.clone(),
        skill_store.clone(),
        weather_client.clone(),
        websearch_client.clone(),
        cfg.web_search.default_num_results,
        &cfg.home,
    );
    niles_tools::register_timer_tools(&mut tools, timers.clone());

    let command_writer = match &cfg.history.directory {
        Some(dir) => {
            let w = CommandWriter::new(dir).context("opening command history writer")?;
            if let Err(e) = w.prune(cfg.history.retention_days) {
                tracing::warn!("history prune failed: {e:#}");
            }
            w
        }
        None => CommandWriter::disabled(),
    };
    let command_writer = Arc::new(command_writer);
    let command_reader = match &cfg.history.directory {
        Some(dir) => Arc::new(CommandReader::new(dir)),
        None => Arc::new(CommandReader::disabled()),
    };
    niles_tools::register_history_tools(&mut tools, command_reader);

    let state_reader = build_state_reader(&cfg);
    niles_tools::register_state_history_tools(&mut tools, state_reader, registry.clone());

    let tier2 = build_tier2_client(&cfg)?;
    if tier2.is_some() {
        niles_tools::register_escalate_tool(&mut tools);
    }

    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(wyoming_bind)
        .await
        .with_context(|| format!("binding Wyoming server on {wyoming_bind}"))?;

    let satellites = Arc::new(SatelliteRegistry::from_config(&cfg.satellites));
    let speakers = Arc::new(speakers::SpeakerRegistry::from_config(&cfg.speakers));

    // Notification center
    let mut notifications = build_notification_center(&cfg.notifications, &cfg.home.timezone);
    let peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let wyoming_delivery = Arc::new(WyomingDelivery {
        piper: piper.clone(),
        sender: server.sender(),
        speakers: speakers.clone(),
        satellites: satellites.clone(),
        peer_index: peer_index.clone(),
    });
    notifications.set_delivery(wyoming_delivery);
    let notifications = Arc::new(notifications);
    niles_tools::register_notification_tools(&mut tools, notifications.clone());

    let tools = Arc::new(tools);

    // Timer driver: shares the `timers` Arc registered with the LLM
    // tools (above) and threaded into `DispatchCtx` (below) so
    // timers set via voice in `serve` actually fire. See
    // `spawn_timer_driver` for behavior + the 60 s sleep-cap caveat.
    let timer_handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

    // Notification subscriber for timer expiry.
    let _timer_notification_handle = {
        let center = notifications.clone();
        let satellites = satellites.clone();
        let mut bus_rx = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match bus_rx.recv().await {
                    Ok(Event::TimerFired { name, origin, .. }) => {
                        let room = satellites.room_for(origin).map(|r| r.as_str().to_string());
                        let text = match name {
                            Some(n) => format!("'{n}' timer finished"),
                            None => "Timer finished".to_string(),
                        };
                        center.deliver(text, room, niles_notifications::Priority::Important);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("notification subscriber lagged by {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    // HTTP API
    let api_state = AppState::new(
        registry.clone(),
        Arc::new(publisher.clone()) as Arc<dyn DevicePublisher>,
        z2m_prefix.clone(),
        bus.clone(),
    );
    let api_handle = tokio::spawn(async move {
        if let Err(e) = niles_api::serve(api_bind, api_state).await {
            tracing::error!("API server exited: {e}");
        }
    });

    // Wyoming + STT + Intent dispatch
    let wyoming_sender = server.sender();
    let server_handle = tokio::spawn(server.run());

    let mode_note = if args.dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "niles serve\n  Z2M:     {prefix}/+/+\n  API:     http://{api_bind}\n  \
         WS:      ws://{api_bind}/events/stream\n  \
         Wyoming: tcp://{wyoming_bind}\n  STT:     {stt_url} ({model})\n  \
         Curve:   tick every {tick}s in {tz}{mode}\nPress Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix,
        stt_url = cfg.stt.base_url,
        model = cfg.stt.model,
        tick = args.tick_seconds.max(1),
        mode = mode_note,
    );

    let n_timers = timers.list().len();
    let n_scenes = scenes.names().len();
    let n_claims = claim_tracker.claimed_count();
    tracing::info!(
        target: "niles::persistence",
        "loaded {n_timers} timers, {n_scenes} scenes, {n_claims} morning claims (dir={:?})",
        cfg.persistence.directory.as_deref(),
    );

    let mut session_tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher: publisher.clone(),
        registry: registry.clone(),
        z2m_prefix: z2m_prefix.clone(),
        dry_run: args.dry_run,
        tracker: tracker.clone(),
        scenes,
        timers,
        speakers,
        llm,
        tier2,
        tools,
        capability_loader,
        capability_index,
        satellites: satellites.clone(),
        device_index,
        history: command_writer,
        memory: memory_store.unwrap_or_else(|| Arc::new(MemoryStore::disabled())),
        skill_store,
        home: Arc::new(cfg.home.clone()),
        review: cfg.skills.review.clone(),
    };

    // Curve loop: driven inline with select! so we share Ctrl-C handling.
    let mut last_published: HashMap<DeviceId, (u8, u16)> = HashMap::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(args.tick_seconds.max(1)));

    let serve_result = loop {
        tokio::select! {
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(room) = satellites.room_for(incoming.from) {
                        peer_index.lock().unwrap_or_else(|e| e.into_inner()).insert(room.clone(), incoming.from);
                    }
                    if let Some(session) = session_tracker.feed(incoming) {
                        spawn_dispatch_task(&whisper, &ctx, &piper, &wyoming_sender, session);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break Ok(());
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    session_tracker.drop_peer(peer);
                    let mut index = peer_index.lock().unwrap_or_else(|e| e.into_inner());
                    index.retain(|_, addr| *addr != peer);
                }
            }
            _ = ticker.tick() => {
                if source_handle.is_finished() {
                    break Err(anyhow::anyhow!(
                        "Z2mSource task has exited; the device registry is no longer \
                         being updated, so the curve would publish blindly. Bailing."
                    ));
                }
                if let Some(routine) = &morning_routine {
                    run_morning_routine_tick(
                        &registry,
                        &publisher,
                        cfg.mqtt.z2m_prefix.as_str(),
                        routine,
                        curve.morning_start,
                        curve.morning_end,
                        tz,
                        args.dry_run,
                        &tracker,
                        &claim_tracker,
                    ).await;
                }
                run_curve_tick(
                    &registry,
                    &publisher,
                    cfg.mqtt.z2m_prefix.as_str(),
                    &curve,
                    tz,
                    args.dry_run,
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                ).await;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break Ok(());
            }
        }
    };

    server_handle.abort();
    source_handle.abort();
    api_handle.abort();
    observer_handle.abort();
    timer_handle.abort();
    serve_result
}

async fn lighting(args: LightingArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = cfg.mqtt.z2m_prefix.clone();

    // Time zone first — fail fast on bad config rather than after
    // we've spun up MQTT and Z2mSource.
    let tz: chrono_tz::Tz = cfg.home.timezone.parse().map_err(|e| {
        anyhow::anyhow!(
            "home.timezone '{}' is not a valid IANA zone: {e}",
            cfg.home.timezone
        )
    })?;
    let curve = cfg
        .lighting
        .to_curve_config()
        .context("converting [lighting] section to a CurveConfig")?;

    let registry = Arc::new(DeviceRegistry::new());
    // Z2mSource requires an EventBus; nothing in this subcommand
    // subscribes to events, so the bus is wired straight in and
    // dropped once Z2mSource takes ownership.
    let ambient_set = build_ambient_set(&cfg);
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        EventBus::default(),
        z2m_prefix.as_str(),
        ambient_set,
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    let tick = Duration::from_secs(args.tick_seconds.max(1));
    let mode = if args.dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "Lighting curve driver running in {tz}{mode}; ticking every {}s. Press Ctrl-C to exit.\n",
        tick.as_secs()
    );

    // Last `(brightness, kelvin)` we published per device. Lets us
    // skip re-sending the same set command while we wait for Z2M to
    // ack the state change — without this, a slow bulb keeps showing
    // its pre-publish brightness and we'd republish every tick.
    let mut last_published: HashMap<DeviceId, (u8, u16)> = HashMap::new();
    let tracker = ManualModeTracker::new();
    let claim_tracker = MorningClaimTracker::new();

    let mut ticker = tokio::time::interval(tick);
    // The first tick fires immediately; that's what we want — no
    // need to wait a full minute on startup before applying the
    // curve to lights that are already on.
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if source_handle.is_finished() {
                    anyhow::bail!(
                        "Z2mSource task has exited; the device registry is no longer \
                         being updated, so the curve would publish blindly. Bailing."
                    );
                }
                run_curve_tick(
                    &registry,
                    &publisher,
                    &z2m_prefix,
                    &curve,
                    tz,
                    args.dry_run,
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                )
                .await;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    source_handle.abort();
    Ok(())
}

/// One tick of the curve driver. Reads the current wall-clock time
/// in `tz`, computes the curve, and publishes (or logs, if `dry_run`)
/// a set command for every currently-on light whose state has
/// drifted past the debounce threshold and whose last-published
/// target differs from the current one.
#[allow(clippy::too_many_arguments)]
async fn run_curve_tick(
    registry: &DeviceRegistry,
    publisher: &MqttPublisher,
    z2m_prefix: &str,
    curve: &niles_scheduler::CurveConfig,
    tz: chrono_tz::Tz,
    dry_run: bool,
    last_published: &mut HashMap<DeviceId, (u8, u16)>,
    tracker: &ManualModeTracker,
    claim_tracker: &MorningClaimTracker,
) {
    let Some((minute_of_day, _)) = current_minute_of_day(tz) else {
        return;
    };
    let target_brightness = brightness_at(curve, minute_of_day);
    let target_kelvin = color_temp_at(curve, minute_of_day);
    let curve_target = (target_brightness, target_kelvin);

    let mut publish_count = 0usize;
    for device in registry.list_all() {
        if device.state.on != Some(true) {
            continue;
        }
        if !device.is_curve_driven() {
            continue;
        }
        if tracker.is_flagged(&device.id) {
            continue;
        }
        if claim_tracker.is_claimed(&device.id) {
            continue;
        }
        // If we already published this exact curve target for this
        // device, skip — even if its reported state hasn't caught
        // up yet (Z2M acks lag the set command).
        if last_published.get(&device.id) == Some(&curve_target) {
            continue;
        }
        let Some(target_state) =
            build_curve_target(&device.state, target_brightness, target_kelvin)
        else {
            continue;
        };
        let (topic, payload) = format_set_command(z2m_prefix, &device.id, &target_state);
        debug_assert!(
            is_actionable(&target_state),
            "build_curve_target should only return actionable targets"
        );
        let ok = if dry_run {
            tracing::info!("[curve {minute_of_day}] [dry-run] {topic}  {payload}");
            true
        } else {
            match publisher.publish(&topic, payload.clone()).await {
                Ok(()) => {
                    tracing::info!("[curve {minute_of_day}] {topic}  {payload}");
                    true
                }
                Err(e) => {
                    tracing::warn!("[curve {minute_of_day}] {topic} failed: {e}");
                    false
                }
            }
        };
        if ok {
            last_published.insert(device.id.clone(), curve_target);
        }
        publish_count += 1;
    }
    tracing::debug!(
        "curve tick at {minute_of_day} in {tz}: brightness={target_brightness}, \
         kelvin={target_kelvin}K, devices_touched={publish_count}"
    );
}

/// One tick of the morning routine. Behaviour:
///
/// - At `morning_start` on a fire-day: claim every off target device
///   and publish `on: true, brightness: 0`.
/// - During the window: for each currently-claimed device, compute the
///   target via `routine_brightness_at` and publish a brightness update
///   if it differs from current state by strictly more than
///   `BRIGHTNESS_DEBOUNCE`.
/// - At `morning_end` or after: release any device the routine still
///   has claimed.
#[allow(clippy::too_many_arguments)]
async fn run_morning_routine_tick(
    registry: &DeviceRegistry,
    publisher: &MqttPublisher,
    z2m_prefix: &str,
    routine: &MorningRoutineConfig,
    morning_start: MinuteOfDay,
    morning_end: MinuteOfDay,
    tz: chrono_tz::Tz,
    dry_run: bool,
    tracker: &ManualModeTracker,
    claim_tracker: &MorningClaimTracker,
) {
    let Some((minute_of_day, now)) = current_minute_of_day(tz) else {
        return;
    };
    let today = now.date_naive();

    // Phase 1 — at exact end-minute, force 100% once then release.
    // After the end-minute, only release leftovers.
    if minute_of_day >= morning_end {
        for id in &routine.target_devices {
            if !claim_tracker.is_claimed(id) {
                continue;
            }

            if minute_of_day == morning_end {
                let Some(device) = registry.get(id) else {
                    claim_tracker.release(id);
                    tracing::info!("[routine {minute_of_day}] released {id}");
                    continue;
                };
                if device.is_curve_driven() && device.state.brightness != Some(100) {
                    let target = DeviceState {
                        brightness: Some(100),
                        ..Default::default()
                    };
                    let (topic, payload) = format_set_command(z2m_prefix, id, &target);
                    if dry_run {
                        tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
                    } else if let Err(e) = publisher.publish(&topic, payload.clone()).await {
                        tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                    } else {
                        tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                    }
                }
            }

            claim_tracker.release(id);
            tracing::info!("[routine {minute_of_day}] released {id}");
        }
        return;
    }
    if minute_of_day < morning_start {
        return;
    }

    // Phase 2 — at start, on a fire-day: claim + kick-on devices that are off.
    let mut just_kicked_on = HashSet::new();
    let firing = should_fire_today(routine, today);
    if minute_of_day == morning_start && firing {
        for id in &routine.target_devices {
            if tracker.is_flagged(id) {
                continue;
            }
            let Some(device) = registry.get(id) else {
                continue;
            };
            if !device.is_curve_driven() {
                continue;
            }
            if device.state.on == Some(true) {
                continue;
            }
            if claim_tracker.is_claimed(id) {
                continue;
            }
            let target = DeviceState {
                on: Some(true),
                brightness: Some(0),
                ..Default::default()
            };
            let (topic, payload) = format_set_command(z2m_prefix, id, &target);
            let ok = if dry_run {
                tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
                true
            } else {
                match publisher.publish(&topic, payload.clone()).await {
                    Ok(()) => {
                        tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                        true
                    }
                    Err(e) => {
                        tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                        false
                    }
                }
            };
            if ok {
                claim_tracker.claim(id);
                just_kicked_on.insert(id.clone());
            }
        }
    }

    // Phase 3 — during window: drive the ramp for claimed devices.
    let Some(target_brightness) = routine_brightness_at(minute_of_day, morning_start, morning_end)
    else {
        return;
    };
    for id in &routine.target_devices {
        if tracker.is_flagged(id) {
            continue;
        }
        if !claim_tracker.is_claimed(id) {
            continue;
        }
        if just_kicked_on.contains(id) {
            continue;
        }
        let Some(device) = registry.get(id) else {
            continue;
        };
        if !device.is_curve_driven() {
            continue;
        }
        let should_publish = match device.state.brightness {
            Some(cur) => cur.abs_diff(target_brightness) > BRIGHTNESS_DEBOUNCE,
            None => true,
        };
        if !should_publish {
            continue;
        }
        let target = DeviceState {
            brightness: Some(target_brightness),
            ..Default::default()
        };
        let (topic, payload) = format_set_command(z2m_prefix, id, &target);
        if dry_run {
            tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
        } else {
            match publisher.publish(&topic, payload.clone()).await {
                Ok(()) => {
                    tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                }
                Err(e) => {
                    tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                }
            }
        }
    }
}

/// Spawn the timer driver task. Wakes on the soonest pending
/// expiry (`TimerStore::next_expiry`), transitions matching
/// timers from `Pending` → `Ringing`, prints a user-visible
/// `[timer] FIRED` line, and publishes [`Event::TimerFired`]
/// for any future consumer (satellite-alarm playback is
/// XVF3800-blocked, out of scope here).
///
/// The sleep is capped at 60 s — without a notify mechanism, a
/// timer added during a long sleep would otherwise miss its
/// deadline. The cap means a shorter timer added late is at
/// worst ~60 s overdue, which is acceptable for v0.1.
///
/// Shared by `voice_dispatch` and `serve`.
fn spawn_timer_driver(timers: Arc<TimerStore>, bus: EventBus) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::time::sleep;
        const MAX_SLEEP: std::time::Duration = std::time::Duration::from_secs(60);
        loop {
            let now = Utc::now();
            let sleep_for = match timers.next_expiry() {
                Some(exp) => {
                    let ms = (exp - now).num_milliseconds().max(0) as u64;
                    std::time::Duration::from_millis(ms).min(MAX_SLEEP)
                }
                None => MAX_SLEEP,
            };
            sleep(sleep_for).await;

            let now = Utc::now();
            for entry in timers
                .list()
                .into_iter()
                .filter(|e| e.is_pending() && e.expires_at <= now)
            {
                if let Some(rung) = timers.mark_ringing(entry.id) {
                    println!(
                        "[timer] FIRED {} (peer={}, id={})",
                        timer_label(&rung),
                        rung.origin,
                        rung.id.0,
                    );
                    bus.publish(Event::TimerFired {
                        id: rung.id.0,
                        name: rung.name.clone(),
                        origin: rung.origin,
                    });
                }
            }
        }
    })
}

/// Presentation helper for a timer entry — lives in the binary, not
/// the scheduler, because user-facing strings are a dispatch concern.
fn timer_label(entry: &TimerEntry) -> String {
    timer_label_for(entry.name.as_deref(), entry.duration)
}

/// Shared shape used both before a timer is stored (`TimerSet` dispatch
/// arm — entry not yet built) and after (`stop`/`list`/`fired`).
fn timer_label_for(name: Option<&str>, duration: std::time::Duration) -> String {
    if let Some(n) = name {
        return format!("'{n}' timer");
    }
    let secs = duration.as_secs();
    if secs >= 3600 {
        format!("{} hour timer", secs / 3600)
    } else if secs >= 60 {
        format!("{} minute timer", secs / 60)
    } else {
        format!("{secs} second timer")
    }
}

/// Compact, human-readable rendering of an [`Intent`] for the dev
/// dispatch log. `{:?}` works but the output is noisier than we want
/// when each line is a single utterance.
fn format_intent(intent: &Intent) -> String {
    // `Intent` is `#[non_exhaustive]` — a future variant lands as
    // `{:?}` until this dev tool catches up.
    match intent {
        Intent::LightSet { room, on } => {
            let state = if *on { "on" } else { "off" };
            format!("LightSet({room} -> {state})")
        }
        Intent::LightSetAll { on } => {
            let state = if *on { "on" } else { "off" };
            format!("LightSetAll(home -> {state})")
        }
        Intent::LightDim { room, percent } => {
            format!("LightDim({room} -> {percent}%)")
        }
        Intent::LightStep {
            room,
            delta_percent,
        } => {
            format!("LightStep({room} {delta_percent:+}%)")
        }
        Intent::LightKelvinStep { room, delta_kelvin } => {
            format!("LightKelvinStep({room} {delta_kelvin:+}K)")
        }
        Intent::LightKelvinSet { room, kelvin } => {
            format!("LightKelvinSet({room} {kelvin}K)")
        }
        Intent::DeviceSet { device_id, on } => {
            let state = if *on { "on" } else { "off" };
            format!("DeviceSet({device_id} -> {state})")
        }
        Intent::DeviceDim { device_id, percent } => {
            format!("DeviceDim({device_id} -> {percent}%)")
        }
        Intent::ClearManualMode { room } => {
            format!("ClearManualMode({})", room.as_deref().unwrap_or("home"))
        }
        Intent::MediaPause { room } => format!("MediaPause({room})"),
        Intent::MediaPlay { room } => format!("MediaPlay({room})"),
        Intent::MediaNext { room } => format!("MediaNext({room})"),
        Intent::MediaPrevious { room } => format!("MediaPrevious({room})"),
        Intent::MediaVolumeSet { room, percent } => format!("MediaVolumeSet({room} -> {percent}%)"),
        Intent::MediaVolumeStep { room, delta } => format!("MediaVolumeStep({room} -> {delta:+})"),
        Intent::TimerSet { duration, name } => match name {
            Some(n) => format!("TimerSet({}s, name={n:?})", duration.as_secs()),
            None => format!("TimerSet({}s)", duration.as_secs()),
        },
        Intent::SceneSave { name, room } => match room {
            Some(r) => format!("SceneSave({name:?} in {r})"),
            None => format!("SceneSave({name:?})"),
        },
        Intent::SceneApply { name } => format!("SceneApply({name:?})"),
        Intent::SceneList => "SceneList".into(),
        Intent::SceneDelete { name } => format!("SceneDelete({name:?})"),
        Intent::TimerCancel { name } => format!("TimerCancel({name:?})"),
        Intent::TimerList => "TimerList".into(),
        Intent::Stop => "Stop".into(),
        Intent::Cancel => "Cancel".into(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod intent_room_tests {
    use super::*;

    #[test]
    fn single_word_room_passes_through() {
        let r = intent_room_to_canonical("kitchen").unwrap();
        assert_eq!(r.as_str(), "kitchen");
    }

    #[test]
    fn multi_word_room_replaces_spaces_with_underscores() {
        let r = intent_room_to_canonical("living room").unwrap();
        assert_eq!(r.as_str(), "living_room");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let r = intent_room_to_canonical("  bedroom  ").unwrap();
        assert_eq!(r.as_str(), "bedroom");
    }

    #[test]
    fn mixed_case_lowercases() {
        // Normalization is this function's contract — callers don't
        // have to pre-lowercase.
        let r = intent_room_to_canonical("Kitchen").unwrap();
        assert_eq!(r.as_str(), "kitchen");
    }

    #[test]
    fn rejects_invalid_chars() {
        // RoomName::parse rejects anything outside [a-z0-9_].
        assert!(intent_room_to_canonical("kitchen!").is_err());
        assert!(intent_room_to_canonical("kitchen-light").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(intent_room_to_canonical("").is_err());
        assert!(intent_room_to_canonical("   ").is_err());
    }
}

#[cfg(test)]
mod timer_label_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn named_timer_ignores_duration() {
        assert_eq!(
            timer_label_for(Some("pasta"), Duration::from_secs(30)),
            "'pasta' timer"
        );
    }

    #[test]
    fn sub_minute_durations_render_in_seconds() {
        // Regression: previously `30s / 60 = 0` gave "0 minute timer".
        assert_eq!(
            timer_label_for(None, Duration::from_secs(30)),
            "30 second timer"
        );
    }

    #[test]
    fn minute_range_renders_in_minutes() {
        assert_eq!(
            timer_label_for(None, Duration::from_secs(60)),
            "1 minute timer"
        );
        assert_eq!(
            timer_label_for(None, Duration::from_secs(8 * 60)),
            "8 minute timer"
        );
    }

    #[test]
    fn hour_range_renders_in_hours() {
        assert_eq!(
            timer_label_for(None, Duration::from_secs(3600)),
            "1 hour timer"
        );
        assert_eq!(
            timer_label_for(None, Duration::from_secs(2 * 3600)),
            "2 hour timer"
        );
    }
}

#[cfg(test)]
mod spawn_timer_driver_tests {
    use super::*;
    use std::net::SocketAddr;

    fn localhost() -> SocketAddr {
        "127.0.0.1:9999".parse().unwrap()
    }

    #[tokio::test]
    async fn driver_transitions_expired_timer_and_publishes_event() {
        let timers = Arc::new(TimerStore::new());
        let bus = EventBus::default();
        // Subscribe before spawning — broadcast::Receiver only sees
        // events published after subscription.
        let mut rx = bus.subscribe();
        let id = timers.set(Duration::from_secs(0), None, localhost(), Utc::now());

        let _handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

        // Deterministic: wait for the event with a generous timeout
        // instead of a fixed sleep that races CI load.
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("driver did not publish TimerFired within 2s")
            .expect("event bus closed unexpectedly");
        match event {
            Event::TimerFired {
                id: fired_id,
                name,
                origin,
            } => {
                assert_eq!(fired_id, id.0);
                assert_eq!(name, None);
                assert_eq!(origin, localhost());
            }
            other => panic!("expected TimerFired, got {other:?}"),
        }

        let entries = timers.list();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].is_ringing(),
            "expected timer {id:?} to be Ringing, got {:?}",
            entries[0].state
        );
    }
}

#[cfg(test)]
mod chat_loop_tests {
    use super::*;
    use niles_llm::{FinishReason, ToolCall};
    use niles_tools::tool::{Tool, ToolDescriptor};
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct FakeChat {
        responses: Mutex<VecDeque<ChatResponse>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl FakeChat {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn captured_requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl niles_llm::LlmBackend for FakeChat {
        async fn chat(&self, req: ChatRequest) -> niles_llm::Result<ChatResponse> {
            self.requests.lock().unwrap().push(req);
            let mut q = self.responses.lock().unwrap();
            q.pop_front()
                .ok_or_else(|| niles_llm::Error::InvalidResponse {
                    reason: "FakeChat ran out of canned responses".into(),
                })
        }
    }

    struct StubTool {
        name: String,
        result: Value,
    }

    #[async_trait::async_trait]
    impl Tool for StubTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: self.name.clone(),
                description: "stub".into(),
                parameters: json!({"type":"object","properties":{},"required":[]}),
            }
        }
        async fn execute(&self, _args: Value) -> niles_tools::Result<Value> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn returns_final_text_when_no_tool_calls() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("hello world".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "hi", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "hello world"));
    }

    #[tokio::test]
    async fn dispatches_tool_call_then_returns_final_text() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "stub".into(),
                    arguments: json!({}),
                }],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("the answer is 42".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"value": 42}),
        }));
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "ask the stub", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "the answer is 42"));
    }

    #[tokio::test]
    async fn errors_when_max_iterations_exhausted() {
        let tool_call_response = || ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "loop".into(),
                name: "stub".into(),
                arguments: json!({}),
            }],
            finish_reason: FinishReason::ToolCalls,
        };
        let fake = FakeChat::new(vec![
            tool_call_response(),
            tool_call_response(),
            tool_call_response(),
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        let err = run_tool_calling_chat(&fake, &registry, "go", None, 3)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("exhausted"),
            "error message should mention exhaustion: got {msg}"
        );
    }

    #[tokio::test]
    async fn empty_content_returns_empty_string() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "hi", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s.is_empty()));
    }

    #[tokio::test]
    async fn tool_error_is_passed_back_as_json_error_message() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "boom".into(),
                    name: "ghost".into(),
                    arguments: json!({}),
                }],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("ok we recovered".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert!(matches!(outcome, LoopOutcome::Done(ref s) if s == "ok we recovered"));
    }

    #[tokio::test]
    async fn trace_records_tool_calls() {
        let fake = FakeChat::new(vec![
            ChatResponse {
                content: None,
                tool_calls: vec![
                    ToolCall {
                        id: "c1".into(),
                        name: "stub".into(),
                        arguments: json!({"x": 1}),
                    },
                    ToolCall {
                        id: "c2".into(),
                        name: "stub".into(),
                        arguments: json!({"x": 2}),
                    },
                ],
                finish_reason: FinishReason::ToolCalls,
            },
            ChatResponse {
                content: Some("done".into()),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
            },
        ]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        let (_outcome, trace) = run_tool_calling_chat(&fake, &registry, "multi", None, 5)
            .await
            .unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].tool, "stub");
        assert_eq!(trace[0].arguments, json!({"x": 1}));
        assert_eq!(trace[0].result, json!({"ok": true}));
        assert_eq!(trace[1].tool, "stub");
        assert_eq!(trace[1].arguments, json!({"x": 2}));
    }

    #[tokio::test]
    async fn system_prompt_is_prepended_to_messages() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        run_tool_calling_chat(&fake, &registry, "hello", Some("YOU ARE NILES"), 5)
            .await
            .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        match reqs[0].messages.as_slice() {
            [Message::System { content }, Message::User { content: user }] => {
                assert_eq!(content, "YOU ARE NILES");
                assert_eq!(user, "hello");
            }
            other => panic!("expected [System, User], got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_system_message_when_system_prompt_is_none() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let registry = ToolRegistry::new();
        run_tool_calling_chat(&fake, &registry, "hello", None, 5)
            .await
            .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        match reqs[0].messages.as_slice() {
            [Message::User { content }] => assert_eq!(content, "hello"),
            other => panic!("expected [User] only, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn escalate_requested_when_model_calls_escalate_tool() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
                arguments: json!({"reason": "too hard"}),
            }],
            finish_reason: FinishReason::ToolCalls,
        }]);
        let registry = ToolRegistry::new();
        let (outcome, _trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert!(
            matches!(outcome, LoopOutcome::EscalateRequested { ref reason, .. } if reason == "too hard"),
            "expected EscalateRequested, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn escalate_mixed_with_other_tools_skips_side_effect_tools() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: None,
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "stub".into(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "c2".into(),
                    name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
                    arguments: json!({"reason": "complex"}),
                },
            ],
            finish_reason: FinishReason::ToolCalls,
        }]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"value": 1}),
        }));
        let (outcome, trace) = run_tool_calling_chat(&fake, &registry, "go", None, 5)
            .await
            .unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].tool, "stub");
        assert_eq!(
            trace[0].result,
            json!({
                "skipped": "deferred_to_tier2",
                "reason": "tier1 requested escalation in this turn"
            })
        );
        assert_eq!(trace[1].tool, niles_tools::escalate::ESCALATE_TOOL_NAME);
        assert_eq!(trace[1].result, json!({"reason": "complex"}));
        assert!(
            matches!(outcome, LoopOutcome::EscalateRequested { ref reason, .. } if reason == "complex"),
            "expected EscalateRequested, got {outcome:?}"
        );
        match &outcome {
            LoopOutcome::EscalateRequested { messages, .. } => {
                assert_eq!(messages.len(), 4); // User + Assistant + Tool(stub) + Tool(escalate)
                match &messages[2] {
                    Message::Tool { content, .. } => assert!(
                        content.contains("deferred_to_tier2"),
                        "expected deferred marker in stub tool response, got {content}"
                    ),
                    other => panic!("expected Tool message, got {other:?}"),
                }
            }
            _ => panic!("expected EscalateRequested"),
        }
    }

    #[tokio::test]
    async fn exclude_tool_removes_it_from_llm_tools() {
        let fake = FakeChat::new(vec![ChatResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
        }]);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool {
            name: "stub".into(),
            result: json!({"ok": true}),
        }));
        // Register a fake escalate tool so it appears in llm_tools
        registry.register(Box::new(StubTool {
            name: niles_tools::escalate::ESCALATE_TOOL_NAME.into(),
            result: json!({"reason": "test"}),
        }));
        let messages = vec![Message::User {
            content: "hi".into(),
        }];
        run_tool_calling_chat_with_messages(
            &fake,
            &registry,
            messages,
            5,
            Some(niles_tools::escalate::ESCALATE_TOOL_NAME),
        )
        .await
        .unwrap();

        let reqs = fake.captured_requests();
        assert_eq!(reqs.len(), 1);
        let tools = reqs[0].tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "stub");
    }
}

#[cfg(test)]
mod system_prompt_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, content: &str) {
        fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    fn fixture_home() -> niles_config::HomeConfig {
        niles_config::HomeConfig {
            name: "Hjemmet".into(),
            latitude: 0.0,
            longitude: 0.0,
            locale: "da_DK".into(),
            timezone: "Europe/Copenhagen".into(),
            country: None,
            units: None,
            default_language: None,
        }
    }

    #[test]
    fn assemble_persona_only_when_loader_is_empty() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_persona_only_when_no_topic_matches() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        // "weather" doesn't intersect with "lighting" or "Control smart lights"
        let out = assemble_system_prompt(
            "what is the weather today",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_injects_matching_capability_body() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: How brightness curves decide brightness\nversion: 1.2.3\n---\n# Lighting Curve\n\nThe curve uses a cosine falloff.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "how does the lighting curve decide brightness",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("The curve uses a cosine falloff."));
        assert!(out.contains("# Capability references"));
    }

    #[test]
    fn assemble_injects_two_matching_capabilities() {
        let tmp = TempDir::new().unwrap();

        let alpha_dir = tmp.path().join("alpha");
        fs::create_dir(&alpha_dir).unwrap();
        write_skill(
            &alpha_dir,
            "---\nname: alpha\ndescription: Alpha capability about scenes\nversion: 1.0.0\n---\nAlpha body.\n",
        );

        let beta_dir = tmp.path().join("beta");
        fs::create_dir(&beta_dir).unwrap();
        write_skill(
            &beta_dir,
            "---\nname: beta\ndescription: Beta capability about scenes\nversion: 2.0.0\n---\nBeta body.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "tell me about scenes",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );

        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("Alpha body."));
        assert!(out.contains("Beta body."));

        // Verify alphabetical ordering: alpha before beta
        let alpha_pos = out.find("Alpha body.").unwrap();
        let beta_pos = out.find("Beta body.").unwrap();
        assert!(
            alpha_pos < beta_pos,
            "alpha should appear before beta in output"
        );
    }

    #[test]
    fn assemble_includes_version_in_section_header() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("my-cap");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: my-cap\ndescription: My capability\nversion: 4.5.6\n---\nBody.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "tell me about my capability",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(
            out.contains("(v4.5.6)"),
            "output should contain version: {out}"
        );
    }

    #[test]
    fn assemble_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert_eq!(a, b, "assemble_system_prompt must be deterministic");
    }

    #[test]
    fn assemble_appends_origin_context_when_present() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("living_room").unwrap();
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Current context"));
        assert!(out.contains("living_room"));
    }

    #[test]
    fn assemble_orders_persona_then_capability_then_context() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("kitchen").unwrap();
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        let persona_pos = out.find(NILES_SYSTEM_PERSONA).unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        let ctx_pos = out.find("# Current context").unwrap();
        assert!(
            persona_pos < cap_pos,
            "persona should come before capability references"
        );
        assert!(
            cap_pos < ctx_pos,
            "capability references should come before current context"
        );
    }

    #[test]
    fn assemble_deterministic_same_inputs() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("bedroom").unwrap();
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with origin_room"
        );
    }

    #[test]
    fn persona_with_origin_none_equals_persona() {
        assert_eq!(persona_with_origin(None), NILES_SYSTEM_PERSONA);
        let room = RoomName::parse("office").unwrap();
        let out = persona_with_origin(Some(&room));
        assert!(out.contains("# Current context"));
        assert!(out.contains("office"));
    }

    #[test]
    fn build_capability_index_round_trips_metadata() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("prereq");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: prereq\ndescription: Needs deps\nversion: 1.0.0\nprerequisites:\n  - foo\n  - bar\n---\nBody.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);

        let entries = index.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "prereq");
        assert_eq!(entries[0].description, "Needs deps");
        assert_eq!(entries[0].prerequisites, vec!["foo", "bar"]);

        // Indirectly verify the capability is detectable and that
        // prerequisite expansion works when the capability itself matches.
        let names = niles_intent::detect_topics("prereq", &index);
        assert!(names.contains(&"prereq".to_string()));
    }

    #[test]
    fn assemble_no_memory_injection_when_none() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_injects_user_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        assert!(out.contains("# User memory"));
        assert!(out.contains("Alice likes tea."));
        assert!(!out.contains("# Agent memory"));
    }

    #[test]
    fn assemble_injects_agent_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            Some("Learned about lighting."),
            None,
        );
        assert!(out.contains("# Agent memory"));
        assert!(out.contains("Learned about lighting."));
        assert!(!out.contains("# User memory"));
    }

    #[test]
    fn assemble_injects_both_memories() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            Some("Learned about lighting."),
            None,
        );
        let user_pos = out.find("# User memory").unwrap();
        let agent_pos = out.find("# Agent memory").unwrap();
        assert!(
            user_pos < agent_pos,
            "user memory should come before agent memory"
        );
    }

    #[test]
    fn assemble_memory_before_capability_references() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        let mem_pos = out.find("# User memory").unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        assert!(
            mem_pos < cap_pos,
            "memory should come before capability references"
        );
    }

    #[test]
    fn assemble_empty_memory_string_is_skipped() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("   "),
            Some(""),
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_no_skills_section_when_summaries_none() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(!out.contains("# Available skills"));
    }

    #[test]
    fn assemble_no_skills_section_when_summaries_empty() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&[]),
        );
        assert!(!out.contains("# Available skills"));
    }

    #[test]
    fn assemble_renders_available_skills_after_memory_before_caps() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![SkillSummary {
            name: "dinner-time".into(),
            description: "Dim lights and start playlist".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            Some(&summaries),
        );
        let user_pos = out.find("# User memory").unwrap();
        let skills_pos = out.find("# Available skills").unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        assert!(
            user_pos < skills_pos,
            "user memory should come before available skills"
        );
        assert!(
            skills_pos < cap_pos,
            "available skills should come before capability references"
        );
        assert!(out.contains("- dinner-time (v0.1.0) — Dim lights and start playlist"));
    }

    #[test]
    fn assemble_skills_list_preserves_incoming_order() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![
            SkillSummary {
                name: "zebra".into(),
                description: "Z".into(),
                version: "1.0.0".into(),
                pinned: false,
                provenance: niles_skills::Provenance::UserCreated,
                status: SkillStatus::Active,
                last_activity_at: None,
            },
            SkillSummary {
                name: "alpha".into(),
                description: "A".into(),
                version: "1.0.0".into(),
                pinned: false,
                provenance: niles_skills::Provenance::UserCreated,
                status: SkillStatus::Active,
                last_activity_at: None,
            },
        ];
        let out = assemble_system_prompt(
            "hello",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        let zebra_pos = out.find("zebra").unwrap();
        let alpha_pos = out.find("alpha").unwrap();
        assert!(
            zebra_pos < alpha_pos,
            "incoming order should be preserved: zebra before alpha"
        );
    }

    #[test]
    fn assemble_is_deterministic_with_skills() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![SkillSummary {
            name: "skill-a".into(),
            description: "Desc".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with skills"
        );
    }

    #[test]
    fn assemble_deterministic_same_inputs_with_skills() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("bedroom").unwrap();
        let summaries = vec![SkillSummary {
            name: "skill-a".into(),
            description: "Desc".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            Some(&summaries),
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            Some(&summaries),
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with origin_room and skills"
        );
    }

    #[test]
    fn assemble_optional_caps_includes_memory_and_skills_without_capabilities() {
        let summaries = vec![SkillSummary {
            name: "dinner-time".into(),
            description: "Dim lights and start playlist".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let out = assemble_system_prompt_with_optional_capabilities(
            "turn on the lights",
            &fixture_home(),
            None,
            None,
            None,
            Some("Alice likes tea."),
            Some("Learned about lighting."),
            Some(&summaries),
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# User memory"));
        assert!(out.contains("# Agent memory"));
        assert!(out.contains("# Available skills"));
        assert!(!out.contains("# Capability references"));
    }

    #[test]
    fn assemble_optional_caps_appends_origin_without_capabilities() {
        let room = RoomName::parse("living_room").unwrap();
        let out = assemble_system_prompt_with_optional_capabilities(
            "hello",
            &fixture_home(),
            None,
            None,
            Some(&room),
            None,
            None,
            None,
        );
        assert!(out.contains("# Current context"));
        assert!(out.contains("living_room"));
    }

    #[test]
    fn system_prompt_renders_stale_annotation() {
        let fixed_now = chrono::Utc::now();
        let summaries = vec![SkillSummary {
            name: "stale-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Stale,
            last_activity_at: Some(fixed_now - chrono::Duration::days(35)),
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(out.contains("[stale: 35 days unused]"));
    }

    #[test]
    fn system_prompt_omits_annotation_for_active() {
        let summaries = vec![SkillSummary {
            name: "active-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(!out.contains("[stale:"));
        assert!(out.contains("- active-skill (v0.1.0) — Does something"));
    }

    #[test]
    fn system_prompt_renders_stale_without_days_when_no_activity() {
        let summaries = vec![SkillSummary {
            name: "stale-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Stale,
            last_activity_at: None,
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(out.contains("[stale]"));
        assert!(!out.contains("days unused"));
    }

    #[test]
    fn spawn_skill_curator_disabled_returns_none() {
        let cfg = niles_config::SkillsCuratorConfig {
            enabled: false,
            ..Default::default()
        };
        // No tokio runtime needed — the enabled branch returns None
        // before any spawn.
        assert!(
            spawn_skill_curator(
                Arc::new(
                    niles_skills::SkillStore::open(
                        std::env::temp_dir().join("niles-test-curator-noop"),
                        100_000,
                        1_048_576,
                    )
                    .unwrap()
                ),
                cfg,
            )
            .is_none()
        );
    }

    #[test]
    fn home_context_includes_all_fields() {
        let home = fixture_home();
        let out = home_context(&home);
        assert!(out.contains("Home: Hjemmet"), "expected home name: {out}");
        assert!(out.contains("Country: DK"), "expected country DK: {out}");
        assert!(
            out.contains("Locale: da_DK"),
            "expected locale da_DK: {out}"
        );
        assert!(
            out.contains("Timezone: Europe/Copenhagen"),
            "expected timezone: {out}"
        );
        assert!(
            out.contains("Units: metric (°C, km)"),
            "expected metric units: {out}"
        );
        assert!(
            out.contains("Spoken language: da"),
            "expected language da: {out}"
        );
    }

    #[test]
    fn home_context_imperial_for_en_us() {
        let mut home = fixture_home();
        home.locale = "en_US".into();
        let out = home_context(&home);
        assert!(
            out.contains("Units: imperial (°F, miles)"),
            "expected imperial units: {out}"
        );
        assert!(out.contains("Country: US"), "expected country US: {out}");
        assert!(
            out.contains("Locale: en_US"),
            "expected locale en_US: {out}"
        );
    }

    #[test]
    fn home_context_unknown_country_when_bare_locale() {
        let mut home = fixture_home();
        home.locale = "en".into();
        home.country = None;
        let out = home_context(&home);
        assert!(out.contains("unknown"), "expected unknown country: {out}");
    }

    #[test]
    fn home_context_sanitizes_control_characters() {
        let mut home = fixture_home();
        home.name = "Hjemmet\n# injected".into();
        home.locale = "da_DK\r\nignore".into();
        home.timezone = "Europe/Copenhagen\t# hidden".into();
        let out = home_context(&home);
        assert!(
            out.contains("Home: Hjemmet # injected"),
            "expected sanitized home field: {out}"
        );
        assert!(
            out.contains("Locale: da_DK ignore"),
            "expected sanitized locale field: {out}"
        );
        assert!(
            out.contains("Timezone: Europe/Copenhagen # hidden"),
            "expected sanitized timezone field: {out}"
        );
    }

    #[test]
    fn system_prompt_includes_household_context_after_persona_before_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        let persona_pos = out.find(NILES_SYSTEM_PERSONA).unwrap();
        let home_idx = out.find("# Household context").unwrap();
        let memory_idx = out.find("# User memory").unwrap();
        assert!(
            persona_pos < home_idx,
            "persona should come before household context"
        );
        assert!(
            home_idx < memory_idx,
            "household context should come before memory"
        );
    }

    #[test]
    fn assemble_system_prompt_is_deterministic_with_home() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with home"
        );
    }
}

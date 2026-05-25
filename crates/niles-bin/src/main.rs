//! niles — AI-first home automation system.

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use niles_api::AppState;
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, RoomName};
use niles_intent::{Intent, IntentRouter};
use niles_llm::{ChatRequest, ChatResponse, GroqClient, GroqConfig, Message, ToolChoice};
use niles_mqtt::{
    MqttClient, MqttOptions, MqttPublisher, Z2mSource, format_set_command, is_actionable,
};
use niles_scheduler::{
    BRIGHTNESS_DEBOUNCE, ManualModeTracker, MinuteOfDay, MorningClaimTracker, MorningRoutineConfig,
    SceneStore, brightness_at, build_curve_target, color_temp_at, routine_brightness_at,
    should_fire_today,
};
use niles_stt::{PcmFormat, WhisperClient, WhisperConfig, pcm_to_wav};
use niles_tools::ToolRegistry;
use niles_tts::{PiperClient, PiperConfig};
use niles_wyoming::{SessionTracker, WyomingServer};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

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
    }
}

async fn config_validate() -> anyhow::Result<()> {
    todo!("implement `config validate`")
}

/// Build an `MqttClient` connected to the broker described in
/// `niles.toml`, with credentials resolved from env vars.
async fn connect_from_config(
    config_path: &std::path::Path,
) -> anyhow::Result<(Config, MqttClient)> {
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

    let source = Z2mSource::new(client, registry.clone(), bus.clone(), &cfg.mqtt.z2m_prefix);

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
    client.publish(&topic, payload.into_bytes()).await?;

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

    let source = Z2mSource::new(client, registry.clone(), bus.clone(), &cfg.mqtt.z2m_prefix);
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    let state = AppState::new(registry.clone());
    let api_handle = tokio::spawn(async move {
        if let Err(e) = niles_api::serve(bind, state).await {
            tracing::error!("API server exited: {e}");
        }
    });

    eprintln!(
        "Z2M source running on {prefix}/+/+; API listening on http://{bind}\n  GET /devices   /rooms/<room>   /healthz\nPress Ctrl-C to exit.",
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

/// A minimal abstraction over the chat-completions endpoint so the
/// tool-calling loop is testable without spinning up an HTTP server.
/// `GroqClient` is the only real implementor; tests use a fake.
#[async_trait::async_trait]
trait ChatProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse>;
}

#[async_trait::async_trait]
impl ChatProvider for GroqClient {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(GroqClient::chat(self, req).await?)
    }
}

/// Drive a chat conversation with tool calling until the model emits
/// a final text response or `max_iterations` is exhausted.
///
/// Each iteration sends the full message history to `client`. If the
/// response carries no tool calls, the function returns the assistant
/// content (or empty string if `content` is `None`). Otherwise the
/// tool calls are dispatched through `registry` and their results are
/// fed back as `Message::Tool` for the next iteration.
async fn run_tool_calling_chat<C: ChatProvider + ?Sized>(
    client: &C,
    registry: &ToolRegistry,
    prompt: &str,
    max_iterations: usize,
) -> anyhow::Result<String> {
    let llm_tools = registry.llm_tools();
    let mut messages = vec![Message::User {
        content: prompt.to_string(),
    }];

    for _ in 0..max_iterations {
        let req = ChatRequest {
            messages: messages.clone(),
            tools: Some(llm_tools.clone()),
            tool_choice: Some(ToolChoice::Auto),
        };
        let resp = client.chat(req).await.context("calling LLM")?;

        if resp.tool_calls.is_empty() {
            return Ok(resp.content.unwrap_or_default());
        }

        messages.push(Message::Assistant {
            content: None,
            tool_calls: Some(resp.tool_calls.clone()),
        });

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

            let result = match registry.execute(call).await {
                Ok(v) => v,
                Err(e) => json!({ "error": format!("{e}") }),
            };
            tracing::info!(
                "tool_result {name} -> {body}",
                name = call.name,
                body = result.to_string()
            );
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

async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = Arc::new(cfg.mqtt.z2m_prefix.clone());

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    tokio::time::sleep(Duration::from_secs(2)).await;

    let tools_registry = niles_tools::default_registry(registry.clone(), publisher, z2m_prefix);
    let client = build_groq_client(&cfg)?;
    eprintln!("Chatting via {} ({}) ...", cfg.llm.base_url, cfg.llm.model);

    // Surface the actual error chain on failure. Both loop exhaustion
    // and a real LLM/network error end up here, but they're distinct
    // failure modes — printing `{e:#}` keeps the cause honest instead
    // of mislabeling every error as "loop exhausted".
    match run_tool_calling_chat(&client, &tools_registry, &args.prompt, MAX_TOOL_ITERATIONS).await {
        Ok(text) => println!("{text}"),
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
) -> Option<(std::net::SocketAddr, String)> {
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

    // Registry populated by Z2mSource. Dispatch tasks look up
    // devices in a room from this shared snapshot.
    let registry = Arc::new(DeviceRegistry::new());
    // Z2mSource requires a bus, but voice-dispatch doesn't subscribe
    // — events are still emitted for any future consumer.
    let bus = EventBus::default();
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    // Build the LLM + tool registry once for the lifetime of the
    // server. Both go into DispatchCtx wrapped in Arc — they're cloned
    // (Arc::clone) into every spawned dispatch task. No Z2M warm-up
    // needed here: the first transcript arrives many seconds after
    // startup (wake-word + speech + STT round-trip), so Z2M has plenty
    // of time to populate before any tool call hits the registry.
    let llm = Arc::new(build_groq_client(&cfg)?);
    let tools = Arc::new(niles_tools::default_registry(
        registry.clone(),
        publisher.clone(),
        z2m_prefix.clone(),
    ));

    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

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

    let server_handle = tokio::spawn(server.run());
    let mut tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher,
        registry: registry.clone(),
        z2m_prefix,
        dry_run: args.dry_run,
        tracker: Arc::new(ManualModeTracker::new()),
        scenes: Arc::new(SceneStore::new()),
        llm,
        tools,
    };

    loop {
        tokio::select! {
            // See voice-tap: drain events before disconnects so a
            // trailing `audio-stop` isn't dropped by the race.
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(session) = tracker.feed(incoming) {
                        // Same unbounded fan-out as voice-tap — fine
                        // for a dev tool, replaced by a bounded
                        // worker pool when this becomes prod dispatch.
                        let whisper = whisper.clone();
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            if let Some((peer, text)) = transcribe_session(&whisper, session).await {
                                handle_transcript(&ctx, peer, &text).await;
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
                    tracker.drop_peer(peer);
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
    llm: Arc<GroqClient>,
    tools: Arc<ToolRegistry>,
}

/// Parse a transcript and act on any Tier 0 intent it produces.
async fn handle_transcript(ctx: &DispatchCtx, peer: std::net::SocketAddr, text: &str) {
    // `transcribe_session` already trims, so an empty `text` here means
    // Whisper returned nothing for a silent/noise session. Don't burn
    // a Groq round-trip on it — Tier 0 wouldn't match either.
    if text.is_empty() {
        tracing::debug!("[{peer}] empty transcript, skipping dispatch");
        return;
    }

    // IntentRouter is a zero-sized unit struct; the regexes are
    // compiled once into a static OnceLock, so constructing one
    // per call is free.
    let intent = match IntentRouter::new().parse(text) {
        Some(i) => i,
        None => {
            // Tier 0 miss — escalate to Tier 1 LLM with the tool registry.
            tracing::info!("[{peer}] Tier 0 miss, escalating to LLM: {text:?}");
            match run_tool_calling_chat(
                ctx.llm.as_ref(),
                ctx.tools.as_ref(),
                text,
                MAX_TOOL_ITERATIONS,
            )
            .await
            {
                Ok(response) => {
                    println!("[{peer}] \"{text}\" -> (Tier 1) {response}");
                }
                Err(e) => {
                    // Mirror the success-path stdout line so a Tier 1
                    // failure is visible without enabling tracing.
                    println!("[{peer}] \"{text}\" -> (Tier 1) error: {e:#}");
                    tracing::warn!("[{peer}] Tier 1 LLM dispatch failed: {e:#}");
                }
            }
            return;
        }
    };

    println!("[{peer}] \"{text}\" -> {}", format_intent(&intent));
    match intent {
        Intent::LightSet { room, on } => {
            let Some((canonical, targets)) =
                resolve_room_targets(ctx, peer, &room, |d| d.state.on.is_some())
            else {
                return;
            };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &canonical, &targets, &desired).await;
        }
        Intent::LightDim { room, percent } => {
            let Some((canonical, targets)) =
                resolve_room_targets(ctx, peer, &room, |d| d.state.brightness.is_some())
            else {
                return;
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
            dispatch_to_targets(ctx, peer, &canonical, &targets, &desired).await;
        }
        Intent::SceneSave { name, room } => {
            let canonical = match room.as_deref().map(intent_room_to_canonical) {
                Some(Ok(r)) => Some(r),
                Some(Err(reason)) => {
                    tracing::warn!(
                        "[{peer}] room {room:?} is not a valid registry name: {reason}",
                        room = room.as_deref().unwrap_or(""),
                    );
                    return;
                }
                None => None,
            };
            let n = ctx.scenes.save(&name, &ctx.registry, canonical.as_ref());
            match &canonical {
                Some(r) => println!("[{peer}] saved scene {name:?} with {n} devices in {r}"),
                None => println!("[{peer}] saved scene {name:?} with {n} devices (whole home)"),
            }
        }
        Intent::SceneApply { name } => {
            let Some(entries) = ctx.scenes.get(&name) else {
                println!("[{peer}] no scene named {name:?}");
                return;
            };
            if entries.is_empty() {
                println!("[{peer}] scene {name:?} is empty — nothing to apply");
                return;
            }
            for entry in entries {
                let (topic, payload) =
                    format_set_command(&ctx.z2m_prefix, &entry.device_id, &entry.state);
                if ctx.dry_run {
                    println!("[{peer}] [dry-run] {topic}  {payload}");
                } else {
                    match ctx
                        .publisher
                        .publish(&topic, payload.as_bytes().to_vec())
                        .await
                    {
                        Ok(()) => println!("[{peer}] published {topic}  {payload}"),
                        Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
                    }
                }
                // ARCHITECTURE.md:501 — scene-applied lights enter
                // manual mode until the user explicitly clears them.
                ctx.tracker.flag(&entry.device_id);
            }
            println!("[{peer}] applied scene {name:?}");
        }
        Intent::ClearManualMode { room } => match room {
            None => {
                let n = ctx.tracker.clear_all();
                println!("[{peer}] back to normal -> cleared manual flag on {n} devices");
            }
            Some(name) => {
                let canonical = match intent_room_to_canonical(&name) {
                    Ok(r) => r,
                    Err(reason) => {
                        tracing::warn!(
                            "[{peer}] room {name:?} is not a valid registry name: {reason}"
                        );
                        return;
                    }
                };
                let n = ctx.tracker.clear_room(&canonical);
                println!(
                    "[{peer}] back to normal in {name} -> cleared manual flag on {n} devices in {canonical}"
                );
            }
        },
        Intent::TimerSet { .. } | Intent::Stop | Intent::Cancel => {
            tracing::info!("{peer}: intent recognized but dispatch not wired yet");
        }
        _ => {
            tracing::info!("{peer}: unknown intent variant, skipping dispatch");
        }
    }
}

/// Resolve a transcript-derived room reference into the canonical
/// `RoomName` + the list of devices in that room that pass
/// `capability_filter`. Returns `None` if the room name is invalid
/// or no devices match — in both cases the right user-facing log
/// line is already emitted here, so the caller just bails.
///
/// Centralizing this avoids the previous duplicated lookup in the
/// `LightDim` arm (one walk to flag, another inside `dispatch_room`
/// to publish).
fn resolve_room_targets<F>(
    ctx: &DispatchCtx,
    peer: std::net::SocketAddr,
    room: &str,
    capability_filter: F,
) -> Option<(RoomName, Vec<niles_core::Device>)>
where
    F: Fn(&niles_core::Device) -> bool,
{
    let canonical = match intent_room_to_canonical(room) {
        Ok(r) => r,
        Err(reason) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            return None;
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
        } else {
            println!(
                "[{peer}] no devices in room '{canonical}' support this action — nothing to dispatch"
            );
        }
        return None;
    }

    Some((canonical, targets))
}

/// Publish the requested target state to each device in `targets`.
/// Pure dispatch — room resolution + capability filtering happened
/// upstream in [`resolve_room_targets`].
async fn dispatch_to_targets(
    ctx: &DispatchCtx,
    peer: std::net::SocketAddr,
    canonical: &RoomName,
    targets: &[niles_core::Device],
    desired: &DeviceState,
) {
    debug_assert!(
        is_actionable(desired),
        "dispatch_to_targets called with a non-actionable target state"
    );
    let _ = canonical; // currently only used for upstream logging; keep the
    // parameter so the caller can pass it without extra plumbing later.

    for device in targets {
        let (topic, payload) = format_set_command(&ctx.z2m_prefix, &device.id, desired);
        if ctx.dry_run {
            println!("[{peer}] [dry-run] {topic}  {payload}");
            continue;
        }
        match ctx
            .publisher
            .publish(&topic, payload.as_bytes().to_vec())
            .await
        {
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
    let normalized: String = s
        .trim()
        .chars()
        .map(|c| match c {
            ' ' | '\t' => '_',
            c => c.to_ascii_lowercase(),
        })
        .collect();
    RoomName::parse(&normalized).map_err(|e| format!("{e}"))
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

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let tracker = Arc::new(ManualModeTracker::new());
    let claim_tracker = Arc::new(MorningClaimTracker::new());

    // Subscribe to the bus *before* spawning the source so we can't miss
    // the early DeviceStateChanged events that seed the observer's
    // last-seen on/off map — broadcast channels only deliver messages
    // sent after a receiver is bound.
    let observer_tracker = tracker.clone();
    let observer_claim_tracker = claim_tracker.clone();
    let mut bus_rx = bus.subscribe();

    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
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
                }
                Ok(Event::DeviceRemoved { id }) => {
                    observer_tracker.forget(&id);
                    observer_claim_tracker.forget(&id);
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
    let tools = Arc::new(niles_tools::default_registry(
        registry.clone(),
        publisher.clone(),
        z2m_prefix.clone(),
    ));

    // HTTP API
    let api_state = AppState::new(registry.clone());
    let api_handle = tokio::spawn(async move {
        if let Err(e) = niles_api::serve(api_bind, api_state).await {
            tracing::error!("API server exited: {e}");
        }
    });

    // Wyoming + STT + Intent dispatch
    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(wyoming_bind)
        .await
        .with_context(|| format!("binding Wyoming server on {wyoming_bind}"))?;
    let server_handle = tokio::spawn(server.run());

    let mode_note = if args.dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "niles serve\n  Z2M:     {prefix}/+/+\n  API:     http://{api_bind}\n  \
         Wyoming: tcp://{wyoming_bind}\n  STT:     {stt_url} ({model})\n  \
         Curve:   tick every {tick}s in {tz}{mode}\nPress Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix,
        stt_url = cfg.stt.base_url,
        model = cfg.stt.model,
        tick = args.tick_seconds.max(1),
        mode = mode_note,
    );

    let mut session_tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher: publisher.clone(),
        registry: registry.clone(),
        z2m_prefix: z2m_prefix.clone(),
        dry_run: args.dry_run,
        tracker: tracker.clone(),
        scenes: Arc::new(SceneStore::new()),
        llm,
        tools,
    };

    // Curve loop: driven inline with select! so we share Ctrl-C handling.
    let mut last_published: HashMap<DeviceId, (u8, u16)> = HashMap::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(args.tick_seconds.max(1)));

    loop {
        tokio::select! {
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(session) = session_tracker.feed(incoming) {
                        let whisper = whisper.clone();
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            if let Some((peer, text)) = transcribe_session(&whisper, session).await {
                                handle_transcript(&ctx, peer, &text).await;
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
                    session_tracker.drop_peer(peer);
                }
            }
            _ = ticker.tick() => {
                if source_handle.is_finished() {
                    anyhow::bail!(
                        "Z2mSource task has exited; the device registry is no longer \
                         being updated, so the curve would publish blindly. Bailing."
                    );
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
                break;
            }
        }
    }

    server_handle.abort();
    source_handle.abort();
    api_handle.abort();
    observer_handle.abort();
    Ok(())
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
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        EventBus::default(),
        z2m_prefix.as_str(),
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
    use chrono::{Timelike, Utc};

    let now = Utc::now().with_timezone(&tz);
    // `chrono::Timelike::hour()` / `minute()` return `u32` but are
    // guaranteed by the trait contract to be `0..=23` / `0..=59`.
    let hour = u8::try_from(now.hour()).expect("chrono::Timelike::hour is 0..=23");
    let minute = u8::try_from(now.minute()).expect("chrono::Timelike::minute is 0..=59");
    let minute_of_day = match MinuteOfDay::new(hour, minute) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("could not construct MinuteOfDay from {now}: {e}");
            return;
        }
    };
    let target_brightness = brightness_at(curve, minute_of_day);
    let target_kelvin = color_temp_at(curve, minute_of_day);
    let curve_target = (target_brightness, target_kelvin);

    let mut publish_count = 0usize;
    for device in registry.list_all() {
        if device.state.on != Some(true) {
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
            match publisher.publish(&topic, payload.as_bytes().to_vec()).await {
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
    use chrono::{Timelike, Utc};

    let now = Utc::now().with_timezone(&tz);
    let hour = u8::try_from(now.hour()).expect("chrono::Timelike::hour is 0..=23");
    let minute = u8::try_from(now.minute()).expect("chrono::Timelike::minute is 0..=59");
    let minute_of_day = match MinuteOfDay::new(hour, minute) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("could not construct MinuteOfDay from {now}: {e}");
            return;
        }
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
                if device.state.brightness != Some(100) {
                    let target = DeviceState {
                        brightness: Some(100),
                        ..Default::default()
                    };
                    let (topic, payload) = format_set_command(z2m_prefix, id, &target);
                    if dry_run {
                        tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
                    } else if let Err(e) =
                        publisher.publish(&topic, payload.as_bytes().to_vec()).await
                    {
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
    let mut just_kicked_on = std::collections::HashSet::new();
    let firing = should_fire_today(routine, today);
    if minute_of_day == morning_start && firing {
        for id in &routine.target_devices {
            if tracker.is_flagged(id) {
                continue;
            }
            let Some(device) = registry.get(id) else {
                continue;
            };
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
                match publisher.publish(&topic, payload.as_bytes().to_vec()).await {
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
        let publish_brightness = match device.state.brightness {
            Some(cur) if cur.abs_diff(target_brightness) > BRIGHTNESS_DEBOUNCE => {
                Some(target_brightness)
            }
            None => Some(target_brightness),
            _ => None,
        };
        let Some(b) = publish_brightness else {
            continue;
        };
        let target = DeviceState {
            brightness: Some(b),
            ..Default::default()
        };
        let (topic, payload) = format_set_command(z2m_prefix, id, &target);
        if dry_run {
            tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
        } else {
            match publisher.publish(&topic, payload.as_bytes().to_vec()).await {
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
        Intent::LightDim { room, percent } => {
            format!("LightDim({room} -> {percent}%)")
        }
        Intent::ClearManualMode { room } => {
            format!("ClearManualMode({})", room.as_deref().unwrap_or("home"))
        }
        Intent::TimerSet { duration, name } => match name {
            Some(n) => format!("TimerSet({}s, name={n:?})", duration.as_secs()),
            None => format!("TimerSet({}s)", duration.as_secs()),
        },
        Intent::SceneSave { name, room } => match room {
            Some(r) => format!("SceneSave({name:?} in {r})"),
            None => format!("SceneSave({name:?})"),
        },
        Intent::SceneApply { name } => format!("SceneApply({name:?})"),
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
mod chat_loop_tests {
    use super::*;
    use niles_llm::{FinishReason, ToolCall};
    use niles_tools::tool::{Tool, ToolDescriptor};
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeChat {
        responses: Mutex<VecDeque<ChatResponse>>,
    }

    impl FakeChat {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ChatProvider for FakeChat {
        async fn chat(&self, _req: ChatRequest) -> anyhow::Result<ChatResponse> {
            let mut q = self.responses.lock().unwrap();
            q.pop_front()
                .ok_or_else(|| anyhow::anyhow!("FakeChat ran out of canned responses"))
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
        let out = run_tool_calling_chat(&fake, &registry, "hi", 5)
            .await
            .unwrap();
        assert_eq!(out, "hello world");
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
        let out = run_tool_calling_chat(&fake, &registry, "ask the stub", 5)
            .await
            .unwrap();
        assert_eq!(out, "the answer is 42");
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
        let err = run_tool_calling_chat(&fake, &registry, "go", 3)
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
        let out = run_tool_calling_chat(&fake, &registry, "hi", 5)
            .await
            .unwrap();
        assert_eq!(out, "");
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
        let out = run_tool_calling_chat(&fake, &registry, "go", 5)
            .await
            .unwrap();
        assert_eq!(out, "ok we recovered");
    }
}

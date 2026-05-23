//! niles — AI-first home automation system.

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use niles_api::AppState;
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, RoomName};
use niles_intent::{Intent, IntentRouter};
use niles_mqtt::{
    MqttClient, MqttOptions, MqttPublisher, Z2mSource, format_set_command, is_actionable,
};
use niles_scheduler::{MinuteOfDay, brightness_at, build_curve_target, color_temp_at};
use niles_stt::{PcmFormat, WhisperClient, WhisperConfig, pcm_to_wav};
use niles_wyoming::{SessionTracker, WyomingServer};
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
    /// Run the main Niles service.
    Serve,
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
    /// on. (v0.1: does not yet respect manual-mode; see PR notes.)
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
struct TranscribeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    config: PathBuf,
    /// Audio file to transcribe (WAV, MP3, FLAC, OGG, M4A, ...).
    audio: PathBuf,
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
        Commands::Serve => todo!("implement `serve` (Phase 2+)"),
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

    let (server, mut rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nPoint your satellite at it. Press Ctrl-C to exit.\n"
    );

    let server_handle = tokio::spawn(async move { server.run().await });

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

async fn voice_tap(args: VoiceTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    let client = Arc::new(build_whisper_client(&cfg)?);
    let (server, mut rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nTranscribing each utterance via {} ({}). Press Ctrl-C to exit.\n",
        cfg.stt.base_url, cfg.stt.model
    );

    let server_handle = tokio::spawn(async move { server.run().await });
    let mut tracker = SessionTracker::new();

    loop {
        tokio::select! {
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
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    // Detached transcription tasks (if any) are dropped here without
    // awaiting — fine for a dev tap; a graceful-shutdown signal will
    // land alongside the connection-close hook.
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

    let (server, mut rx) = WyomingServer::bind(bind)
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

    let server_handle = tokio::spawn(async move { server.run().await });
    let mut tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher,
        registry: registry.clone(),
        z2m_prefix,
        dry_run: args.dry_run,
    };

    loop {
        tokio::select! {
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
}

/// Parse a transcript and act on any Tier 0 intent it produces.
async fn handle_transcript(ctx: &DispatchCtx, peer: std::net::SocketAddr, text: &str) {
    // IntentRouter is a zero-sized unit struct; the regexes are
    // compiled once into a static OnceLock, so constructing one
    // per call is free.
    let intent = match IntentRouter::new().parse(text) {
        Some(i) => i,
        None => {
            println!("[{peer}] \"{text}\" -> no Tier 0 match");
            return;
        }
    };

    println!("[{peer}] \"{text}\" -> {}", format_intent(&intent));
    match intent {
        Intent::LightSet { room, on } => {
            dispatch_light_set(ctx, peer, &room, on).await;
        }
        Intent::TimerSet { .. } | Intent::Stop | Intent::Cancel => {
            tracing::info!("{peer}: intent recognized but dispatch not wired yet");
        }
        _ => {
            tracing::info!("{peer}: unknown intent variant, skipping dispatch");
        }
    }
}

/// Resolve a transcript-derived room name to actionable devices in
/// the registry, then publish a Z2M `set` command toggling each.
async fn dispatch_light_set(ctx: &DispatchCtx, peer: std::net::SocketAddr, room: &str, on: bool) {
    let canonical = match intent_room_to_canonical(room) {
        Ok(r) => r,
        Err(reason) => {
            tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
            return;
        }
    };

    // Filter to devices that expose a power-state field. A bare
    // sensor will never have `on: Some(_)` in its current state,
    // so this skips them without needing a separate capability
    // model.
    let targets: Vec<_> = ctx
        .registry
        .list_room(&canonical)
        .into_iter()
        .filter(|d| d.state.on.is_some())
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
                "[{peer}] no controllable devices in room '{canonical}' — nothing to dispatch"
            );
        }
        return;
    }

    let desired = DeviceState {
        on: Some(on),
        ..Default::default()
    };
    debug_assert!(
        is_actionable(&desired),
        "LightSet target should always be actionable"
    );

    for device in &targets {
        let (topic, payload) = format_set_command(&ctx.z2m_prefix, &device.id, &desired);
        if ctx.dry_run {
            println!("[{peer}] [dry-run] {topic}  {payload}");
            continue;
        }
        match ctx
            .publisher
            .publish(&topic, payload.clone().into_bytes())
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
async fn run_curve_tick(
    registry: &DeviceRegistry,
    publisher: &MqttPublisher,
    z2m_prefix: &str,
    curve: &niles_scheduler::CurveConfig,
    tz: chrono_tz::Tz,
    dry_run: bool,
    last_published: &mut HashMap<DeviceId, (u8, u16)>,
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
        if dry_run {
            tracing::info!("[curve {minute_of_day}] [dry-run] {topic}  {payload}");
            last_published.insert(device.id.clone(), curve_target);
        } else {
            match publisher.publish(&topic, payload.as_bytes().to_vec()).await {
                Ok(()) => {
                    tracing::info!("[curve {minute_of_day}] {topic}  {payload}");
                    last_published.insert(device.id.clone(), curve_target);
                }
                Err(e) => tracing::warn!("[curve {minute_of_day}] {topic} failed: {e}"),
            }
        }
        publish_count += 1;
    }
    tracing::debug!(
        "curve tick at {minute_of_day} in {tz}: brightness={target_brightness}, \
         kelvin={target_kelvin}K, devices_touched={publish_count}"
    );
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
        Intent::TimerSet { duration, name } => match name {
            Some(n) => format!("TimerSet({}s, name={n:?})", duration.as_secs()),
            None => format!("TimerSet({}s)", duration.as_secs()),
        },
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

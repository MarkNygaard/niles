//! niles — AI-first home automation system.

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use niles_api::AppState;
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_mqtt::{MqttClient, MqttOptions, Z2mSource, format_set_command, is_actionable};
use niles_stt::{PcmFormat, WhisperClient, WhisperConfig, pcm_to_wav};
use niles_wyoming::{SessionTracker, WyomingServer};
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
                            transcribe_session(&client, session).await;
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

/// Wrap a session's PCM in WAV and ship it to Whisper. Errors are
/// logged rather than propagated so a single bad request doesn't
/// take the listener down.
async fn transcribe_session(client: &WhisperClient, session: niles_wyoming::AudioSession) {
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
            return;
        }
    };

    match client.transcribe(wav, "session.wav").await {
        Ok(t) => {
            println!("[{}] \"{}\"", session.from, t.text.trim());
        }
        Err(e) => {
            tracing::warn!("{}: transcription failed: {e}", session.from);
        }
    }
}

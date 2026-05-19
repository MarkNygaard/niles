//! niles — AI-first home automation system.

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_mqtt::{MqttClient, MqttOptions, Z2mSource, format_set_command, is_actionable};
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
    #[arg(long)]
    brightness: Option<u8>,
    /// Color temperature in Kelvin.
    #[arg(long)]
    kelvin: Option<u16>,
    /// Print the message that would be published without actually sending.
    #[arg(long)]
    dry_run: bool,
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

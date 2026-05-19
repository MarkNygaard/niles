//! niles — AI-first home automation system.

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use niles_config::Config;
use niles_mqtt::{MqttClient, MqttOptions};
use std::path::PathBuf;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env at repo root for local dev. Silently ignored if absent.
    let _ = dotenvy::dotenv();

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
    }
}

async fn config_validate() -> anyhow::Result<()> {
    todo!("implement `config validate`")
}

async fn mqtt_tap(args: MqttTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
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
    let mut client = MqttClient::connect(opts);

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
                    eprintln!("\nMQTT event loop terminated. Exiting.");
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

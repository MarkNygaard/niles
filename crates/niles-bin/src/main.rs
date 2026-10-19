//! niles — AI-first home automation system.

use crate::cli::{Cli, Commands, ConfigAction};
use crate::commands::{
    api, chat, config_validate, discover, enroll, mqtt_tap, set, synthesize, transcribe, voice_tap,
    wyoming_tap,
};
use crate::lighting::lighting;
use crate::manifest::generate_manifest;
use crate::serve::serve;
use crate::voice::voice_dispatch;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod app;
mod approvals;
mod artifacts;
mod assistant;
mod calendar;
mod chat_log;
mod chat_loop;
mod chatgpt;
mod claude_code;
mod cli;
mod climate;
mod commands;
mod conversation;
mod courtesy;
mod dispatch;
mod follow_up;
mod grocery_delivery;
mod last_target;
mod lighting;
mod mail;
mod manifest;
mod music;
mod plugins;
mod profile;
mod prompt;
mod push;
mod recognition;
mod response;
mod review;
mod ringer;
mod satellites;
mod serve;
mod speak;
mod speakers;
mod speech_stream;
mod timers;
mod tool_servers;
mod training;
mod tv;
mod vocabulary;
mod voice;
mod wake_up;
mod wiring;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env at repo root for local dev. Silently ignored if absent.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    // Initialize tracing. Honors RUST_LOG; defaults to `info` for the
    // niles_* crates so dev subcommands give meaningful output.
    // `niles plugin dev` also shows the plugin's timers firing.
    let plugins_level = match &cli.command {
        Commands::Plugin {
            action: plugins::PluginAction::Dev(_),
        } => "debug",
        _ => "info",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // The binary target is named `niles`, so its module path is
            // `niles::` — an earlier `niles_bin=` directive matched nothing
            // and silently swallowed every log line from this file.
            EnvFilter::new(format!(
                "niles=info,niles_api=info,niles_config=info,niles_db=info,niles_llm=info,niles_mqtt=info,niles_nemlig=info,niles_plugins={plugins_level},niles_presence=info,niles_scheduler=info,niles_speakers=info,niles_stt=info,niles_tools=info,niles_webos=info",
            ))
        });
    // The same events go to stdout and to a small in-memory ring the
    // HTTP API serves. `kubectl logs` is a different tool with
    // different credentials, and "why did it just do that" is asked far
    // too often to live behind one.
    // Process-wide, like the subscriber that fills it — threading it
    // through every subcommand's signature to reach the one that serves
    // HTTP would be ceremony for a thing there is exactly one of.
    let logs = LOG_BUFFER
        .get_or_init(niles_api::logs::LogBuffer::new)
        .clone();
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(niles_api::logs::LogLayer::new(logs.clone()))
        .init();

    match cli.command {
        Commands::Serve(args) => serve(args).await,
        Commands::Config { action } => match action {
            ConfigAction::Validate(args) => config_validate(args),
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
        Commands::Enroll(args) => enroll(args).await,
        Commands::Lighting(args) => lighting(args).await,
        Commands::GenerateManifest(args) => generate_manifest(args),
        Commands::Plugin { action } => plugins::run(action).await,
    }
}

/// The in-memory log ring, installed at startup and served by `/logs`.
static LOG_BUFFER: std::sync::OnceLock<niles_api::logs::LogBuffer> = std::sync::OnceLock::new();

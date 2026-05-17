//! niles — AI-first home automation system.

use clap::{Parser, Subcommand};

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve => todo!("implement `serve` (Phase 2+)"),
        Commands::MigrateFromHa => todo!("implement `migrate-from-ha`"),
        Commands::FlashSatellite => todo!("implement `flash-satellite`"),
        Commands::Config { action } => match action {
            ConfigAction::Validate => todo!("implement `config validate`"),
        },
        Commands::Tools { action } => match action {
            ToolsAction::List => todo!("implement `tools list`"),
        },
    }
}

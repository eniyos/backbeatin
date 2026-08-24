// Binary-only modules (CLI parsing and command implementations).
mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

/// The default config filename changed from `backbeat.toml` to
/// `backbeatin.toml`. When the new default is used but only the legacy
/// file exists, fall back to it with a deprecation notice.
fn resolve_config_path(config: PathBuf) -> PathBuf {
    if config.as_os_str() == "backbeatin.toml" && !config.exists() {
        let legacy = PathBuf::from("backbeat.toml");
        if legacy.exists() {
            tracing::warn!("backbeat.toml is deprecated — rename it to backbeatin.toml");
            return legacy;
        }
    }
    config
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise logging: respect RUST_LOG if set, otherwise default to INFO.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Verify {
            repo,
            config,
            db_path,
            sample,
        } => {
            let config = resolve_config_path(config);
            commands::verify::run_verify(&config, &repo, &db_path, sample.as_deref()).await?;
        }
        Commands::Daemon { config, db_path } => {
            let config = resolve_config_path(config);
            commands::daemon::run_daemon(&config, &db_path).await?;
        }
        Commands::Demo { output } => {
            commands::demo::run_demo(&output).await?;
        }
    }

    Ok(())
}

// Binary-only modules (CLI parsing and command implementations).
mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

/// Default filenames changed from `backbeat.*` to `backbeatin.*`. When the
/// new default is used but only the legacy file exists, fall back to it with
/// a deprecation notice.
fn resolve_default_path(path: PathBuf, new_name: &str, legacy_name: &str) -> PathBuf {
    if path.as_os_str() == new_name && !path.exists() {
        let legacy = PathBuf::from(legacy_name);
        if legacy.exists() {
            tracing::warn!("{legacy_name} is deprecated — rename it to {new_name}");
            return legacy;
        }
    }
    path
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
            let config = resolve_default_path(config, "backbeatin.toml", "backbeat.toml");
            let db_path = resolve_default_path(db_path, "backbeatin.db", "backbeat.db");
            commands::verify::run_verify(&config, &repo, &db_path, sample.as_deref()).await?;
        }
        Commands::Daemon { config, db_path } => {
            let config = resolve_default_path(config, "backbeatin.toml", "backbeat.toml");
            let db_path = resolve_default_path(db_path, "backbeatin.db", "backbeat.db");
            commands::daemon::run_daemon(&config, &db_path).await?;
        }
        Commands::Demo { output } => {
            commands::demo::run_demo(&output).await?;
        }
    }

    Ok(())
}

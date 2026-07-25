mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};
use tracing_subscriber::EnvFilter;

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
        Commands::Verify { repo, config, db_path } => {
            commands::verify::run_verify(&config, &repo, &db_path).await?;
        }
    }

    Ok(())
}

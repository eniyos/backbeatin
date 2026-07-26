use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Backbeatin — automatically verify that your Restic and Borg backups can
/// actually be restored.
///
/// Reads an existing backup repository (read-only), performs a real restore
/// into an ephemeral sandbox, cryptographically verifies the result, and
/// records a signed proof of each run.
#[derive(Parser, Debug)]
#[command(name = "backbeat", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the daemon scheduler for periodic verification.
    Daemon {
        /// Path to the TOML configuration file.
        #[arg(short, long, default_value = "backbeat.toml")]
        config: PathBuf,

        /// Path to the SQLite database for persisting run history.
        #[arg(long, default_value = "backbeat.db")]
        db_path: PathBuf,
    },
    /// Verify a repository by performing a real restore.
    Verify {
        /// Name of the repository in the config file to verify.
        repo: String,

        /// Path to the TOML configuration file.
        #[arg(short, long, default_value = "backbeat.toml")]
        config: PathBuf,

        /// Path to the SQLite database for persisting run history.
        #[arg(long, default_value = "backbeat.db")]
        db_path: PathBuf,
    },
}

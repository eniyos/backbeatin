use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Backbeatin — Backup Verification Tool
///
/// Automatically verify that your Restic and Borg backups can actually be restored.
///
/// This tool reads an existing backup repository (read-only), performs a real restore
/// into an ephemeral Docker sandbox, cryptographically verifies the result, and
/// records a signed proof of each run.
///
/// ## Quick Start
///
/// 1. Create a configuration file (backbeatin.toml)
/// 2. Run verification: backbeatin verify <repo-name>
/// 3. Start daemon: backbeatin daemon
///
/// ## Example Configuration
///
/// ```toml
/// [[repo]]
/// name = "prod-backup"
/// backend = "restic"
/// uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/path"
/// schedule = "0 0 * * * *"  # Every hour
///
/// [repo.credential_env_vars]
/// AWS_ACCESS_KEY_ID = "AWS access key"
/// AWS_SECRET_ACCESS_KEY = "AWS secret key"
/// ```
#[derive(Parser, Debug)]
#[command(name = "backbeatin", version, about, long_about = None)]
#[command(author = "eniyos")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Verify a repository by performing a real restore
    ///
    /// Performs a complete verification cycle: snapshot discovery, sandboxed restore,
    /// manifest computation, and cryptographic verification. Results are persisted
    /// to the local database and signed with your Ed25519 key.
    ///
    /// ## Example
    ///
    /// ```bash
    /// backbeatin verify prod-backup -c backbeatin.toml
    /// ```
    ///
    /// ## Exit Codes
    ///
    /// - 0: Verification passed
    /// - 1: Verification failed or error occurred
    Verify {
        /// Name of the repository to verify (from config file)
        #[arg(help = "Repository name as defined in backbeatin.toml")]
        repo: String,

        /// Path to the TOML configuration file
        #[arg(
            short,
            long,
            default_value = "backbeatin.toml",
            help = "Path to configuration file"
        )]
        config: PathBuf,

        /// Path to the `SQLite` database for persisting run history
        #[arg(long, default_value = "backbeat.db", help = "Path to SQLite database")]
        db_path: PathBuf,

        /// Restore only a sample instead of the full snapshot.
        ///
        /// Either a percentage ("10" or "10%") selecting a deterministic
        /// hash-based subset of files, or a path glob ("*/logs/*.log")
        /// selecting matching files. Use for large repos between full
        /// restores.
        #[arg(
            long,
            value_name = "PERCENT|GLOB",
            help = "Sample restore: percentage (e.g. '10') or path glob (e.g. '*/logs/*')"
        )]
        sample: Option<String>,
    },

    /// Run the daemon scheduler for periodic verification
    ///
    /// Starts the background scheduler that continuously verifies repositories
    /// according to their configured cron schedules. Sends webhook notifications
    /// on verification failures. Runs until interrupted (Ctrl+C).
    ///
    /// ## Example
    ///
    /// ```bash
    /// backbeatin daemon -c backbeatin.toml
    /// ```
    ///
    /// ## Signals
    ///
    /// - Ctrl+C: Gracefully shutdown
    Daemon {
        /// Path to the TOML configuration file
        #[arg(
            short,
            long,
            default_value = "backbeatin.toml",
            help = "Path to configuration file"
        )]
        config: PathBuf,

        /// Path to the `SQLite` database for persisting run history
        #[arg(long, default_value = "backbeat.db", help = "Path to SQLite database")]
        db_path: PathBuf,
    },

    /// Run a self-contained demo against a synthetic backup repository
    ///
    /// Creates a healthy backup (verify passes), corrupts the repository
    /// (verify fails), and exports a signed proof bundle. Useful for testing
    /// the verification pipeline without affecting real backups.
    ///
    /// ## Example
    ///
    /// ```bash
    /// backbeatin demo -o proof-bundle.json
    /// ```
    ///
    /// ## Requirements
    ///
    /// - Docker must be running
    /// - Requires the restic Docker image
    Demo {
        /// Output path for the proof bundle JSON
        #[arg(
            short,
            long,
            default_value = "demo-proof-bundle.json",
            help = "Output file for proof bundle"
        )]
        output: PathBuf,
    },
}

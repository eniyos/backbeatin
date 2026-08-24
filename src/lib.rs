//! # Backbeatin Core Library
//!
//! This library provides the core functionality for backup verification,
//! including configuration management, backup backend abstraction,
//! Docker sandboxing, cryptographic signing, and persistence.
//!
//! ## Architecture
//!
//! The library is organized into several key modules:
//!
//! - **config**: Configuration parsing and validation (TOML-based)
//! - **repo**: Backup backend abstraction (Restic, Borg)
//! - **sandbox**: Docker container management for isolated restores
//! - **sign**: Ed25519 cryptographic signing for tamper-evident records
//! - **store**: `SQLite` persistence for verification history
//! - **verify**: Manifest computation and verification logic
//! - **notify**: Webhook notification system for alerts
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use std::path::Path;
//! use backbeatin::{Config, Store, ResticBackend, BackupBackend};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Load configuration
//! let config = Config::load(Path::new("backbeatin.toml"))?;
//!
//! // Open persistence store
//! let store = Store::open(Path::new("backbeatin.db"))?;
//!
//! // Initialize backend
//! let backend = ResticBackend::from_config(&config.repos[0])?;
//!
//! // Get latest snapshot
//! let snapshot_id = backend.latest_snapshot_id().await?;
//!
//! // Perform restore and verification
//! // ... restore logic ...
//! # Ok(())
//! # }
//! ```
//!
//! ## Thread Safety
//!
//! The `Store` struct uses internal mutex locking to support concurrent
//! access from multiple threads, making it safe for use in async contexts
//! like the daemon mode cron scheduler.

pub mod config;
pub mod repo;
pub mod sample;
pub mod sandbox;
pub mod sign;
pub mod store;
pub mod verify;

pub use config::{BackendType, Config, RepoConfig};
pub use repo::{BackupBackend, BorgBackend, ListedFile, RepoStats, ResticBackend, RestoreOutcome};
pub use sample::{parse_sample_spec, scope_label, select_files, SampleSpec};
pub use sandbox::Sandbox;
pub use sign::{manifest_sha256, run_signing_message, Signer};
pub use store::{unix_now, NewVerificationRun, Store, VerificationRunRecord};
pub use verify::{
    compute_manifest, verify_restore, Manifest, ManifestEntry, VerificationResult,
    VerificationStatus,
};

pub mod notify;
pub use notify::Notifier;

// Re-export sandbox image constants used by the CLI commands.
pub use sandbox::{DEFAULT_IMAGE_BORG, DEFAULT_IMAGE_RESTIC};

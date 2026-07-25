use std::path::Path;

use anyhow::Context;
use backbeat_core::{
    verify_restore, compute_manifest, Config, RepoConfig, ResticBackend, BackupBackend,
    VerificationStatus,
};

/// Execute the `verify` subcommand.
///
/// 1. Load configuration from `config_path`.
/// 2. Find `RepoConfig` by `repo_name`.
/// 3. Instantiate the correct backend.
/// 4. Create a temp directory and restore the latest snapshot into it.
/// 5. Compute a manifest of restored files (SHA-256 + size).
/// 6. Compare the manifest against the backend-reported outcome.
/// 7. Print pass/fail and exit with the appropriate code.
pub async fn run_verify(config_path: &Path, repo_name: &str) -> anyhow::Result<()> {
    let config = Config::load(config_path)
        .context("Failed to load configuration")?;

    let repo_config = config
        .repos
        .iter()
        .find(|r| r.name == repo_name)
        .ok_or_else(|| anyhow::anyhow!(
            "Repository '{}' not found in config file '{}'",
            repo_name,
            config_path.display()
        ))?;

    run_restore(repo_config).await?;

    Ok(())
}

/// Perform the actual restore and verification for a single repo config.
async fn run_restore(config: &RepoConfig) -> anyhow::Result<()> {
    let backend = ResticBackend::from_config(config)
        .context("Failed to initialise Restic backend from config")?;

    // --- Step 1: discover latest snapshot ---
    tracing::info!("Looking up latest snapshot for repo '{}'…", config.name);
    let snapshot_id = backend
        .latest_snapshot_id()
        .await
        .context("Failed to discover latest snapshot ID")?;
    tracing::info!("Latest snapshot: {}", snapshot_id);

    // --- Step 2: restore into temp directory ---
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    tracing::info!("Restoring snapshot {} into {:?}…", snapshot_id, tmp_dir.path());

    let outcome = backend
        .restore_snapshot(&snapshot_id, tmp_dir.path())
        .await
        .context("Restore operation failed")?;
    tracing::info!(
        "Restore completed: {} files, {} bytes",
        outcome.files_count,
        outcome.bytes_restored,
    );

    // --- Step 3: compute manifest ---
    tracing::info!("Computing manifest of restored files…");
    let manifest = compute_manifest(tmp_dir.path())
        .context("Failed to compute file manifest from restored data")?;
    tracing::info!(
        "Manifest computed: {} entries, {} total bytes",
        manifest.total_files,
        manifest.total_bytes,
    );

    // --- Step 4: verify ---
    let result = verify_restore(&outcome, &manifest);

    match result.status {
        VerificationStatus::Pass => {
            tracing::info!("✅ VERIFICATION PASSED: {}", result.message);
            println!("{}", result.message);
            Ok(())
        }
        VerificationStatus::Fail => {
            tracing::error!("❌ VERIFICATION FAILED: {}", result.message);
            eprintln!("ERROR: {}", result.message);
            // Exit with a non-zero code for scripting convenience.
            std::process::exit(1);
        }
    }
}

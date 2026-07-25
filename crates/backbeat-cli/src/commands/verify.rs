use std::path::Path;

use anyhow::Context;
use backbeat_core::{
    compute_manifest, verify_restore, BackupBackend, Config, NewVerificationRun, RepoConfig,
    ResticBackend, Sandbox, Store, VerificationStatus,
};

/// Execute the `verify` subcommand.
///
/// 1. Load config and open the store (SQLite DB).
/// 2. Find the `RepoConfig` by `repo_name`.
/// 3. Create a temp directory and restore the latest snapshot into it.
/// 4. Compute a manifest of restored files (SHA-256 + size).
/// 5. Compare the manifest against the backend-reported outcome.
/// 6. Persist the run to the store.
/// 7. Print pass/fail and exit with the appropriate code.
pub async fn run_verify(
    config_path: &Path,
    repo_name: &str,
    db_path: &Path,
) -> anyhow::Result<()> {
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

    let store = Store::open(db_path).context("Failed to open store database")?;

    run_restore(repo_config, &store).await?;

    Ok(())
}

/// Perform the actual restore, verification, and persistence.
async fn run_restore(config: &RepoConfig, store: &Store) -> anyhow::Result<()> {
    let backend = ResticBackend::from_config(config)
        .context("Failed to initialise Restic backend from config")?;

    let started_at = backbeat_core::store::unix_now();

    // --- Step 1: discover latest snapshot ---
    tracing::info!("Looking up latest snapshot for repo '{}'…", config.name);
    let snapshot_id = backend
        .latest_snapshot_id()
        .await
        .context("Failed to discover latest snapshot ID")?;
    tracing::info!("Latest snapshot: {}", snapshot_id);

    // --- Step 2: restore into temp directory via Docker sandbox ---
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    tracing::info!(
        "Restoring snapshot {} into {:?} (Docker sandbox)…",
        snapshot_id,
        tmp_dir.path(),
    );

    let sandbox = Sandbox::connect()
        .await
        .context("Failed to connect to Docker for sandbox restore")?;
    sandbox.ensure_image().await?;

    let restore_stdout = sandbox
        .run_restic_restore(config, &snapshot_id, tmp_dir.path())
        .await
        .context("Sandbox restore failed")?;

    let outcome = ResticBackend::parse_restore_output(&restore_stdout, &snapshot_id)
        .context("Failed to parse restic restore output from container")?;
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

    let completed_at = backbeat_core::store::unix_now();

    // --- Step 4: verify ---
    let result = verify_restore(&outcome, &manifest);

    // --- Step 5: persist to store ---
    let status_str = match result.status {
        VerificationStatus::Pass => "pass".to_string(),
        VerificationStatus::Fail => "fail".to_string(),
    };
    let run_record = NewVerificationRun {
        repo_name: config.name.clone(),
        repo_backend: format!("{:?}", config.backend).to_lowercase(),
        repo_uri: config.uri.clone(),
        snapshot_id: snapshot_id.clone(),
        status: result.status,
        files_count: outcome.files_count,
        bytes_restored: outcome.bytes_restored,
        message: result.message.clone(),
        manifest: Some(manifest),
        started_at,
        completed_at,
    };

    if let Err(e) = store.insert_verification_run(&run_record) {
        tracing::warn!("Failed to persist verification run: {}", e);
    } else {
        tracing::info!("Run persisted to store ({})", status_str);
    }

    // --- Step 6: report result ---
    match result.status {
        VerificationStatus::Pass => {
            tracing::info!("✅ VERIFICATION PASSED: {}", result.message);
            println!("{}", result.message);
            Ok(())
        }
        VerificationStatus::Fail => {
            tracing::error!("❌ VERIFICATION FAILED: {}", result.message);
            // Return an error so destructors (TempDir, DB connection) run
            // properly before the process exits with non-zero status.
            Err(anyhow::anyhow!("{}", result.message))
        }
    }
}

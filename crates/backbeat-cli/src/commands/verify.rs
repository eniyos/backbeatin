use std::path::Path;

use anyhow::Context;
use backbeat_core::{
    compute_manifest, verify_restore, BackendType, BorgBackend, BackupBackend, Config,
    NewVerificationRun, RepoConfig, ResticBackend, RestoreOutcome, Sandbox, Store,
    VerificationStatus,
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
    let started_at = backbeat_core::store::unix_now();

    // --- Step 1: discover latest snapshot ---
    tracing::info!("Looking up latest snapshot for repo '{}'…", config.name);

    let snapshot_id = match config.backend {
        BackendType::Restic => {
            let backend = ResticBackend::from_config(config)
                .context("Failed to initialise Restic backend from config")?;
            backend.latest_snapshot_id().await
                .context("Failed to discover latest snapshot ID")?
        }
        BackendType::Borg => {
            let backend = BorgBackend::from_config(config)
                .context("Failed to initialise Borg backend from config")?;
            backend.latest_snapshot_id().await
                .context("Failed to discover latest Borg archive name")?
        }
    };
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

    let (outcome, _restore_stdout) = match config.backend {
        BackendType::Restic => {
            let sb = sandbox.with_image(backbeat_core::sandbox::DEFAULT_IMAGE_RESTIC);
            sb.ensure_image().await?;
            let stdout = sb
                .run_restic_restore(config, &snapshot_id, tmp_dir.path())
                .await
                .context("Sandbox restic restore failed")?;
            let outcome = ResticBackend::parse_restore_output(&stdout, &snapshot_id)
                .context("Failed to parse restic restore JSON output from container")?;
            (outcome, Some(stdout))
        }
        BackendType::Borg => {
            let sb = sandbox.with_image(backbeat_core::sandbox::DEFAULT_IMAGE_BORG);
            sb.ensure_image().await?;
            let stdout = sb
                .run_borg_extract(config, &snapshot_id, tmp_dir.path())
                .await
                .context("Sandbox Borg extract failed")?;
            // Borg does not produce JSON output for extract — return zero
            // stats and let the manifest-based verification do the real check.
            let outcome = RestoreOutcome {
                snapshot_id: snapshot_id.clone(),
                files_count: 0,
                bytes_restored: 0,
            };
            (outcome, Some(stdout))
        }
    };

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

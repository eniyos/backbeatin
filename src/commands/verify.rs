//! Single-shot verification command implementation.
//!
//! This module handles the `backbeat verify` subcommand which performs
//! a one-time verification of a backup repository.
//!
//! # Verification Flow
//!
//! 1. **Configuration Loading**: Load and validate the TOML configuration
//! 2. **Repository Selection**: Find the specified repository configuration
//! 3. **Snapshot Discovery**: Identify the latest snapshot to verify
//! 4. **Sandboxed Restore**: Restore the snapshot into an ephemeral Docker container
//! 5. **Manifest Computation**: Compute SHA-256 hashes of all restored files
//! 6. **Verification**: Compare manifest against backend-reported statistics
//! 7. **Persistence**: Store the verification result in `SQLite` database
//! 8. **Signing**: Cryptographically sign the verification record
//! 9. **Reporting**: Print pass/fail status and exit with appropriate code
//!
//! # Exit Codes
//!
//! - `0`: Verification passed successfully
//! - `1`: Verification failed or error occurred

use std::path::Path;

use anyhow::Context;
use backbeatin::{
    config::{BackendType, Config, RepoConfig, SandboxConfig},
    notify::Notifier,
    repo::{BackupBackend, BorgBackend, ListedFile, ResticBackend, RestoreOutcome},
    sample::{parse_sample_spec, scope_label, select_files},
    sandbox::Sandbox,
    sign::Signer,
    store::{unix_now, NewVerificationRun, Store},
    verify::{compute_manifest, verify_restore, VerificationResult, VerificationStatus},
};
use backbeatin::{manifest_sha256, run_signing_message, DEFAULT_IMAGE_BORG, DEFAULT_IMAGE_RESTIC};

/// Execute the `verify` subcommand.
///
/// Performs a complete verification cycle for a single repository:
/// 1. Load config and open the store (`SQLite` DB)
/// 2. Find the `RepoConfig` by `repo_name`
/// 3. Create a temp directory and restore the latest snapshot into it
/// 4. Compute a manifest of restored files (SHA-256 + size)
/// 5. Compare the manifest against the backend-reported outcome
/// 6. Persist the run to the store
/// 7. Print pass/fail and exit with the appropriate code
pub async fn run_verify(
    config_path: &Path,
    repo_name: &str,
    db_path: &Path,
    sample: Option<&str>,
) -> anyhow::Result<()> {
    let config = Config::load(config_path).context("Failed to load configuration")?;

    let repo_config = config
        .repos
        .iter()
        .find(|r| r.name == repo_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Repository '{}' not found in config file '{}'",
                repo_name,
                config_path.display()
            )
        })?;

    let store = Store::open(db_path).context("Failed to open store database")?;

    // run_restore returns Err on verification failure, so reaching the next
    // line means the run succeeded.
    run_restore(repo_config, &store, sample, &config.sandbox).await?;

    // Dead man's switch: ping the heartbeat endpoint on success so an
    // external watchdog (Healthchecks.io, Cronitor, …) can alert when
    // single-shot runs stop happening.
    if let Some(notifier) = Notifier::from_config(&config) {
        if let Err(e) = notifier.send_heartbeat(repo_name).await {
            tracing::warn!("Failed to send heartbeat: {}", e);
        }
    }

    Ok(())
}

/// Perform the actual restore, verification, and persistence.
/// Returns the `VerificationResult` for notification purposes.
///
/// When `sample_spec` is `Some`, only a subset of the snapshot is restored
/// (a percentage of files or a path glob) — see [`backbeatin::sample`].
#[allow(clippy::too_many_lines)]
pub async fn run_restore(
    config: &RepoConfig,
    store: &Store,
    sample_spec: Option<&str>,
    sandbox_limits: &SandboxConfig,
) -> anyhow::Result<VerificationResult> {
    let started_at = unix_now();

    // --- Step 1: discover latest snapshot ---
    tracing::info!("Looking up latest snapshot for repo '{}'…", config.name);

    let snapshot_id = match config.backend {
        BackendType::Restic => {
            let backend = ResticBackend::from_config(config)
                .context("Failed to initialise Restic backend from config")?;
            backend
                .latest_snapshot_id()
                .await
                .context("Failed to discover latest snapshot ID")?
        }
        BackendType::Borg => {
            let backend = BorgBackend::from_config(config)
                .context("Failed to initialise Borg backend from config")?;
            backend
                .latest_snapshot_id()
                .await
                .context("Failed to discover latest Borg archive name")?
        }
    };
    tracing::info!("Latest snapshot: {}", snapshot_id);

    // --- Step 1b: plan sampling (if requested) ---
    let includes: Vec<String> = if let Some(spec) = sample_spec {
        let parsed = parse_sample_spec(spec)?;
        let files: Vec<ListedFile> = match config.backend {
            BackendType::Restic => ResticBackend::from_config(config)?
                .list_snapshot_files(&snapshot_id)
                .await
                .context("Failed to list snapshot contents for sampling")?,
            BackendType::Borg => BorgBackend::from_config(config)?
                .list_snapshot_files(&snapshot_id)
                .await
                .context("Failed to list archive contents for sampling")?,
        };
        let selected = select_files(&files, &parsed);
        if selected.is_empty() {
            anyhow::bail!(
                "Sample spec '{spec}' selected 0 of {} files — nothing to verify",
                files.len()
            );
        }
        let selected_bytes: u64 = selected.iter().map(|f| f.size).sum();
        tracing::info!(
            "Sampling: {} of {} files selected ({} bytes) via spec '{spec}'",
            selected.len(),
            files.len(),
            selected_bytes
        );
        selected.into_iter().map(|f| f.path).collect()
    } else {
        Vec::new()
    };
    let scope = scope_label(sample_spec);

    // --- Step 2: restore into temp directory via Docker sandbox ---
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    tracing::info!(
        "Restoring snapshot {} into {:?} (Docker sandbox)…",
        snapshot_id,
        tmp_dir.path(),
    );

    let sandbox = Sandbox::connect()
        .context("Failed to connect to Docker for sandbox restore")?
        .with_limits(sandbox_limits);

    let outcome = match config.backend {
        BackendType::Restic => {
            let sb = sandbox.with_image(DEFAULT_IMAGE_RESTIC);
            sb.ensure_image().await?;
            let stdout = sb
                .run_restic_restore(config, &snapshot_id, tmp_dir.path(), &includes)
                .await
                .context("Sandbox restic restore failed")?;
            ResticBackend::parse_restore_output(&stdout, &snapshot_id)
                .context("Failed to parse restic restore JSON output from container")?
        }
        BackendType::Borg => {
            let sb = sandbox.with_image(DEFAULT_IMAGE_BORG);
            sb.ensure_image().await?;
            let _stdout = sb
                .run_borg_extract(config, &snapshot_id, tmp_dir.path(), &includes)
                .await
                .context("Sandbox Borg extract failed")?;
            // Borg does not produce JSON output for extract — return zero
            // stats and let the manifest-based verification do the real check.
            RestoreOutcome {
                snapshot_id: snapshot_id.clone(),
                files_count: 0,
                bytes_restored: 0,
                count_is_meaningful: false,
            }
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

    let completed_at = unix_now();

    // --- Step 4: verify ---
    // Load the manifest of the last successful run of this snapshot *with
    // the same restore scope* (if any) so drift detection can catch
    // same-size content corruption without confusing sampled and full runs.
    let baseline = store
        .last_successful_manifest(&config.name, &snapshot_id, scope.as_deref())
        .context("Failed to load baseline manifest for drift detection")?;
    if baseline.is_some() {
        tracing::info!(
            "Baseline manifest found for snapshot {} — drift detection active",
            snapshot_id
        );
    }
    let result = verify_restore(&outcome, &manifest, baseline.as_ref());

    // --- Step 5: persist to store ---
    let status_str = match result.status {
        VerificationStatus::Pass => "pass".to_string(),
        VerificationStatus::Fail => "fail".to_string(),
    };
    let run_record = NewVerificationRun {
        repo_name: config.name.clone(),
        repo_backend: config.backend.to_string(),
        repo_uri: config.uri.clone(),
        snapshot_id: snapshot_id.clone(),
        status: result.status,
        files_count: outcome.files_count,
        bytes_restored: outcome.bytes_restored,
        message: result.message.clone(),
        manifest: Some(manifest),
        signature_hex: None,
        public_key_hex: None,
        restore_scope: scope,
        started_at,
        completed_at,
    };

    // Insert and sign the run record.
    match store.insert_verification_run(&run_record) {
        Ok(run_id) => {
            tracing::info!("Run persisted to store ({})", status_str);

            // Compute the manifest hash (SHA-256 of the JSON-serialized manifest).
            let manifest_hash = run_record
                .manifest
                .as_ref()
                .map(manifest_sha256)
                .unwrap_or_default();

            // Sign the run data (best-effort).
            if let Ok(signer) = Signer::auto_load_or_generate() {
                if let Ok(msg) = run_signing_message(
                    run_id,
                    &config.name,
                    &snapshot_id,
                    &status_str,
                    outcome.files_count,
                    outcome.bytes_restored,
                    &result.message,
                    &manifest_hash,
                    started_at,
                    completed_at,
                ) {
                    let sig = signer.sign(&msg);
                    let pk_hex = signer.public_key_hex();
                    if let Err(e) = store.update_run_signature(run_id, &sig, &pk_hex) {
                        tracing::warn!("Failed to persist signature: {}", e);
                    } else {
                        tracing::info!("Run signed with Ed25519 key");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to persist verification run: {}", e);
        }
    }

    // --- Step 6: report result ---
    match result.status {
        VerificationStatus::Pass => {
            tracing::info!("✅ VERIFICATION PASSED: {}", result.message);
            println!("{}", result.message);
            Ok(result)
        }
        VerificationStatus::Fail => {
            tracing::error!("❌ VERIFICATION FAILED: {}", result.message);
            // Return an error so destructors (TempDir, DB connection) run
            // properly before the process exits with non-zero status.
            Err(anyhow::anyhow!("{}", result.message))
        }
    }
}

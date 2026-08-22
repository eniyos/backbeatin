use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use tokio_cron_scheduler::{Job, JobScheduler};

use backbeatin::{config::Config, notify::Notifier, store::Store, verify::{VerificationResult, VerificationStatus, Manifest}};

/// Run the daemon: continuously verify repositories on their configured
/// schedules.
///
/// Reads the config, creates a cron job per repository, and keeps running
/// until the process is interrupted (Ctrl+C).
pub async fn run_daemon(config_path: &Path, db_path: &Path) -> anyhow::Result<()> {
    let config = Config::load(config_path).context("Failed to load configuration")?;

    // Open the store once and share it across all cron jobs via Arc.
    // Store uses internal Mutex<Connection> so it is safe for concurrent access.
    let store = Arc::new(
        Store::open(db_path).context("Failed to open store database")?,
    );

    let notifier = Notifier::from_config(&config);
    let config_path = config_path.to_owned();

    let sched = JobScheduler::new().await?;

    for repo in &config.repos {
        let cron_expr = repo.schedule.clone();
        let repo_name = repo.name.clone();
        let job_repo_name = repo_name.clone();
        let cp = config_path.clone();
        let st = Arc::clone(&store);
        let nf = notifier.clone();

        let job = Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let rn = job_repo_name.clone();
            let cp = cp.clone();
            let st = Arc::clone(&st);
            let nf = nf.clone();

            Box::pin(async move {
                tracing::info!("[{}] Starting scheduled verification…", rn);

                // Load config (fresh copy) and find this repo.
                let config = match Config::load(&cp) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("[{}] Failed to load config: {}", rn, e);
                        return;
                    }
                };

                let repo_config = match config.repos.iter().find(|r| r.name == rn) {
                    Some(r) => r.clone(),
                    None => {
                        tracing::error!("[{}] Repo not found in config", rn);
                        return;
                    }
                };

                // Run the actual verification (store is shared via Arc).
                match super::verify::run_restore(&repo_config, &st).await {
                    Ok(result) => {
                        tracing::info!("[{}] Verification passed", rn);
                        // Send notification on success if configured (rare).
                        if let Some(ref notifier) = nf {
                            if let Err(e) = notifier.send(&rn, &result).await {
                                tracing::warn!("[{}] Failed to send success notification: {}", rn, e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("[{}] Verification failed: {}", rn, e);
                        // Send failure notification with the error message.
                        if let Some(ref notifier) = nf {
                            let result = VerificationResult {
                                status: VerificationStatus::Fail,
                                message: e.to_string(),
                                manifest: Manifest::default(),
                            };
                            if let Err(notify_err) = notifier.send(&rn, &result).await {
                                tracing::error!("[{}] Failed to send failure notification: {}", rn, notify_err);
                            }
                        }
                    }
                }
            })
        })?;

        sched.add(job).await?;
        tracing::info!(
            "[{}] Scheduled: cron='{}'",
            repo_name,
            cron_expr,
        );
    }

    tracing::info!(
        "Daemon started with {} repo(s). Press Ctrl+C to stop.",
        config.repos.len(),
    );

    // Start the scheduler and wait for shutdown signal.
    sched.start().await?;

    // Wait indefinitely until interrupted.
    tokio::signal::ctrl_c().await?;
    tracing::info!("Received shutdown signal, exiting…");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_daemon_config_parsing() {
        let tmp = tempdir().expect("Failed to create temp dir");
        let config_path = tmp.path().join("test.toml");
        let db_path = tmp.path().join("test.db");

        let config_content = r#"
[[repo]]
name = "test-repo"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/test"
"#;

        fs::write(&config_path, config_content).expect("Failed to write config");

        // Test that the config can be loaded (no credential env vars needed)
        let config = Config::load(&config_path).expect("Failed to load config");
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].name, "test-repo");

        // Test that the store can be opened
        let store = Store::open(&db_path).expect("Failed to open store");
        drop(store);

        // Clean up
        fs::remove_file(&config_path).ok();
        fs::remove_file(&db_path).ok();
    }
}

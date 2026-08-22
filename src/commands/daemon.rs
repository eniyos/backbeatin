use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use tokio_cron_scheduler::{Job, JobScheduler};

use backbeatin::{
    config::{Config, RepoConfig},
    notify::Notifier,
    store::{unix_now, Store},
    verify::{Manifest, VerificationResult, VerificationStatus},
};

/// Run the daemon: continuously verify repositories on their configured
/// schedules.
///
/// Reads the config, creates a cron job per repository, and keeps running
/// until the process is interrupted (Ctrl+C).
pub async fn run_daemon(config_path: &Path, db_path: &Path) -> anyhow::Result<()> {
    let config = Config::load(config_path).context("Failed to load configuration")?;

    // Open the store once and share it across all cron jobs via Arc.
    // Store uses internal Mutex<Connection> so it is safe for concurrent access.
    let store = Arc::new(Store::open(db_path).context("Failed to open store database")?);

    let notifier = Notifier::from_config(&config);
    let config_path = config_path.to_owned();

    let sched = JobScheduler::new().await?;

    for repo in &config.repos {
        register_repo_jobs(&sched, repo, &config_path, &store, notifier.as_ref()).await?;
    }

    // Dead man's switch (daemon side): if a staleness window is configured,
    // periodically check that every repo has had a successful run recently.
    // This catches silently-skipped jobs while the daemon is alive; a dead
    // daemon/host is only catchable via the external heartbeat_url watchdog.
    arm_staleness_switch(&sched, &config, &store, notifier.as_ref()).await?;

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

/// Register the cron jobs for a single repository: the regular schedule
/// (sampled when `sample` is configured) and, if configured, a separate
/// full-restore cadence.
///
/// # Errors
///
/// Returns an error if a cron job cannot be created or scheduled.
async fn register_repo_jobs(
    sched: &JobScheduler,
    repo: &RepoConfig,
    config_path: &Path,
    store: &Arc<Store>,
    notifier: Option<&Notifier>,
) -> anyhow::Result<()> {
    let schedules: Vec<(String, bool)> = {
        let mut v = vec![(repo.schedule.clone(), false)];
        if let Some(full) = &repo.full_schedule {
            v.push((full.clone(), true));
        }
        v
    };

    for (cron_expr, is_full_run) in schedules {
        let job_repo_name = repo.name.clone();
        let cp = config_path.to_owned();
        let st = Arc::clone(store);
        let nf = notifier.cloned();

        let job = Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let rn = job_repo_name.clone();
            let cp = cp.clone();
            let st = Arc::clone(&st);
            let nf = nf.clone();

            Box::pin(async move {
                run_scheduled_verification(&rn, is_full_run, &cp, &st, nf.as_ref()).await;
            })
        })?;

        sched.add(job).await?;
        tracing::info!(
            "[{}] Scheduled: cron='{}' ({})",
            repo.name,
            cron_expr,
            if is_full_run { "full" } else { "regular" },
        );
    }

    Ok(())
}

/// Execute one scheduled verification for `repo_name` and notify the
/// outcome (including the heartbeat ping on success).
async fn run_scheduled_verification(
    rn: &str,
    is_full_run: bool,
    config_path: &Path,
    store: &Arc<Store>,
    notifier: Option<&Notifier>,
) {
    tracing::info!(
        "[{}] Starting scheduled {} verification…",
        rn,
        if is_full_run { "full" } else { "sampled" }
    );

    // Load config (fresh copy) and find this repo.
    let config = match Config::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[{}] Failed to load config: {}", rn, e);
            return;
        }
    };

    let repo_config = if let Some(r) = config.repos.iter().find(|r| r.name == rn) {
        r.clone()
    } else {
        tracing::error!("[{}] Repo not found in config", rn);
        return;
    };

    // Full-cadence runs ignore the sample spec; regular runs
    // use it when configured.
    let sample_spec = if is_full_run {
        None
    } else {
        repo_config.sample.as_deref()
    };

    // Run the actual verification (store is shared via Arc).
    match super::verify::run_restore(&repo_config, store, sample_spec, &config.sandbox).await {
        Ok(result) => {
            tracing::info!("[{}] Verification passed", rn);
            // Send notification on success if configured (rare).
            if let Some(notifier) = notifier {
                if let Err(e) = notifier.send(rn, &result).await {
                    tracing::warn!("[{}] Failed to send success notification: {}", rn, e);
                }
                // Dead man's switch: ping the heartbeat endpoint
                // so an external watchdog can alert if successes
                // stop arriving.
                if let Err(e) = notifier.send_heartbeat(rn).await {
                    tracing::warn!("[{}] Failed to send heartbeat: {}", rn, e);
                }
            }
        }
        Err(e) => {
            tracing::error!("[{}] Verification failed: {}", rn, e);
            // Send failure notification with the error message.
            if let Some(notifier) = notifier {
                let result = VerificationResult {
                    status: VerificationStatus::Fail,
                    message: e.to_string(),
                    manifest: Manifest::default(),
                };
                if let Err(notify_err) = notifier.send(rn, &result).await {
                    tracing::error!(
                        "[{}] Failed to send failure notification: {}",
                        rn,
                        notify_err
                    );
                }
            }
        }
    }
}

/// Register the dead man's switch staleness job, if a window is configured.
///
/// # Errors
///
/// Returns an error if the staleness job cannot be created or scheduled.
async fn arm_staleness_switch(
    sched: &JobScheduler,
    config: &Config,
    store: &Arc<Store>,
    notifier: Option<&Notifier>,
) -> anyhow::Result<()> {
    let Some(notif_cfg) = config.notifications.as_ref() else {
        return Ok(());
    };
    let Some(max_hours) = notif_cfg.max_success_age_hours else {
        return Ok(());
    };

    let repo_names: Vec<String> = config.repos.iter().map(|r| r.name.clone()).collect();
    let st = Arc::clone(store);
    let nf = notifier.cloned();
    let daemon_started_at = unix_now();
    let window_secs = max_hours.saturating_mul(3600).cast_signed();

    let staleness_job = Job::new_async("0 */30 * * * *", move |_uuid, _lock| {
        let names = repo_names.clone();
        let st = Arc::clone(&st);
        let nf = nf.clone();

        Box::pin(async move {
            let now = unix_now();
            for name in &names {
                // Repos that never passed are measured against daemon
                // start time so a fresh install isn't instantly stale.
                let reference = match st.last_success_at(name) {
                    Ok(Some(ts)) => ts,
                    Ok(None) => daemon_started_at,
                    Err(e) => {
                        tracing::warn!("[{}] Staleness check failed: {}", name, e);
                        continue;
                    }
                };
                if now - reference <= window_secs {
                    continue;
                }

                let message = format!(
                    "Dead man's switch: repo '{name}' has had no successful \
                     verification in the last {max_hours} hour(s) — silent \
                     non-execution suspected",
                );
                tracing::error!("[{}] {}", name, message);

                if let Some(ref notifier) = nf {
                    let result = VerificationResult {
                        status: VerificationStatus::Fail,
                        message,
                        manifest: Manifest::default(),
                    };
                    if let Err(e) = notifier.send(name, &result).await {
                        tracing::error!("[{}] Failed to send staleness alert: {}", name, e);
                    }
                }
            }
        })
    })?;

    sched.add(staleness_job).await?;
    tracing::info!(
        "Dead man's switch armed: alert if no successful run within {} hour(s)",
        max_hours
    );
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

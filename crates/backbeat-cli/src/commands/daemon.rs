use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use tokio_cron_scheduler::{Job, JobScheduler};

use backbeat_core::{Config, Notifier, Store};

/// Run the daemon: continuously verify repositories on their configured
/// schedules.
///
/// Reads the config, creates a cron job per repository, and keeps running
/// until the process is interrupted (Ctrl+C).
pub async fn run_daemon(config_path: &Path, db_path: &Path) -> anyhow::Result<()> {
    let config = Arc::new(
        Config::load(config_path).context("Failed to load configuration")?,
    );

    // Open the store once so we verify the DB is accessible early.
    let _store = Store::open(db_path).context("Failed to open store database")?;
    let db_path = db_path.to_owned();

    let notifier = Notifier::from_config(&config);
    let config_path = config_path.to_owned();

    let sched = JobScheduler::new().await?;

    for repo in &config.repos {
        let cron_expr = repo.schedule.clone();
        let repo_name = repo.name.clone();
        let job_repo_name = repo_name.clone();
        let cp = config_path.clone();
        let dp = db_path.clone();
        let nf = notifier.clone();

        let job = Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let rn = job_repo_name.clone();
            let cp = cp.clone();
            let dp = dp.clone();
            let nf = nf.clone();

            Box::pin(async move {
                let rn = rn.clone();
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

                let store = match Store::open(&dp) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("[{}] Failed to open store: {}", rn, e);
                        return;
                    }
                };

                // Placeholder — actual verify flow will be wired next.
                tracing::info!(
                    "[{}] Verification for {} backend at {} will run here",
                    rn,
                    format!("{:?}", repo_config.backend).to_lowercase(),
                    repo_config.uri,
                );
                let _ = (&store, &nf);
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

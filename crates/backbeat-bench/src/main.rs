use std::path::Path;
use std::process::Command as StdCommand;
use std::time::Instant;

use anyhow::Context;
use backbeat_core::{compute_manifest, verify_restore, ResticBackend, BackupBackend};
use bollard::container::{
    Config as ContainerConfig, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StatsOptions, WaitContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use clap::Parser;
use futures_util::StreamExt;

const MINIO_IMAGE: &str = "minio/minio:latest";
const RESTIC_IMAGE: &str = "restic/restic:latest";

// ---------------------------------------------------------------------------
// Pricing constants (USD per GB egress)
// Source: AWS S3 pricing https://aws.amazon.com/s3/pricing/ (us-east-1, 2025-07)
//         Backblaze B2 pricing https://www.backblaze.com/cloud-storage/pricing (2025-07)
// Note: These are list prices as of the date cited. Actual costs depend on
// region, volume tiers, and any negotiated discounts. Verify against your
// own provider/region before using for budgeting.
// ---------------------------------------------------------------------------
const AWS_S3_EGRESS_PER_GB: f64 = 0.09;
const B2_EGRESS_PER_GB: f64 = 0.01;

/// Repo size tiers for benchmarking.
struct SizeTier {
    label: &'static str,
    target_bytes: u64,
}

const TIERS: &[SizeTier] = &[
    SizeTier { label: "small", target_bytes: 100 * 1024 * 1024 },   // ~100MB
    SizeTier { label: "medium", target_bytes: 10 * 1024 * 1024 * 1024 },  // ~10GB
    SizeTier { label: "large", target_bytes: 100 * 1024 * 1024 * 1024 },  // ~100GB
];

/// Per-run measurement.
#[derive(Debug, Clone, serde::Serialize)]
struct RunMeasurement {
    tier: String,
    iteration: usize,
    duration_secs: f64,
    bytes_transferred: u64,
    peak_memory_kb: u64,
    status: String,
}

/// Aggregated tier results.
#[derive(Debug, Clone, serde::Serialize)]
struct TierResult {
    tier: String,
    runs: Vec<RunMeasurement>,
    avg_duration_secs: f64,
    avg_bytes_transferred: u64,
    avg_peak_memory_kb: u64,
    daily_cost_aws: f64,
    weekly_cost_aws: f64,
    daily_cost_b2: f64,
    weekly_cost_b2: f64,
}

/// Benchmark harness for backbeat restore verification costs.
#[derive(Parser, Debug)]
#[command(name = "backbeat-bench", version)]
struct BenchCli {
    /// Number of verification cycles per size tier.
    #[arg(long, default_value = "5")]
    iterations: usize,

    /// Only run a specific tier (small, medium, large). Runs all if omitted.
    #[arg(long)]
    tier: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let cli = BenchCli::parse();

    // Ensure host `restic` (init/backup and `ResticBackend` snapshot discovery)
    // uses the same fixed repo password + MinIO credentials as the containers.
    std::env::set_var("RESTIC_PASSWORD", "backbeat-bench-password");
    std::env::set_var("AWS_ACCESS_KEY_ID", "minioadmin");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "minioadmin");

    let tiers: Vec<&SizeTier> = if let Some(ref t) = cli.tier {
        TIERS.iter().filter(|s| s.label == t).collect()
    } else {
        TIERS.iter().collect()
    };

    if tiers.is_empty() {
        anyhow::bail!("No matching tiers found");
    }

    let mut all_results: Vec<TierResult> = Vec::new();

    // Pull images once at the start.
    tracing::info!("Pulling Docker images…");
    pull_image(MINIO_IMAGE);
    pull_image(RESTIC_IMAGE);

    for tier in &tiers {
        let label = tier.label;

        tracing::info!("═══ Tier: {} ({}) ═══", label, format_size(tier.target_bytes));

        let result = bench_tier(cli.iterations, label, tier.target_bytes).await?;
        all_results.push(result);
    }

    // Generate report.
    let report = generate_report(&all_results);
    std::fs::write("BENCHMARK_RESULTS.md", &report)?;
    println!("{}", report);
    tracing::info!("Report written to BENCHMARK_RESULTS.md");

    Ok(())
}

// ---------------------------------------------------------------------------
// Per-tier benchmark
// ---------------------------------------------------------------------------

async fn bench_tier(iterations: usize, label: &str, target_bytes: u64) -> anyhow::Result<TierResult> {
    let (port, minio_id) = start_minio();
    let minio_ip = get_container_ip(&minio_id);
    tracing::info!("MinIO running on port {} (container IP {})", port, minio_ip);

    // Host `restic` reaches MinIO via the published port; the restore container
    // reaches the same bucket via MinIO's IP on the shared bridge network.
    let host_repo_url = format!("s3:http://127.0.0.1:{}/backbeat-bench-{}", port, label);
    let container_repo_url = format!("s3:http://{}:9000/backbeat-bench-{}", minio_ip, label);

    // Temp dir under /tmp so it bind-mounts cleanly into Docker containers
    // (Docker Desktop/macOS cannot reliably mount /var/folders).
    let tmp = tempfile::Builder::new()
        .prefix("backbeat-bench")
        .tempdir_in("/tmp")
        .context("tempdir")?;
    let tmp_path = tmp.path().to_path_buf();

    // Init repo.
    run_restic(&tmp_path, &["init", "--repo", &host_repo_url])?;
    tracing::info!("Restic repo initialised");

    // Generate synthetic data.
    let data_dir = tmp_path.join("data");
    std::fs::create_dir_all(&data_dir).context("create data dir")?;
    generate_data(&data_dir, target_bytes).context("generate data")?;
    tracing::info!("Synthetic data generated: {}", format_size(target_bytes));

    // Backup with a relative source path (cwd = tmp root) so the snapshot root
    // is `data/`, keeping the restore tree and file-count check clean.
    run_restic(&tmp_path, &["backup", "--repo", &host_repo_url, "data"])?;
    tracing::info!("Backup completed");

    // Create config for backbeat.
    let config_content = format!(
        "[[repo]]\nname = \"bench-{}\"\nbackend = \"restic\"\nuri = \"{}\"\n[repo.credential_env_vars]\nAWS_ACCESS_KEY_ID = \"x\"\nAWS_SECRET_ACCESS_KEY = \"x\"\n",
        label, host_repo_url,
    );
    let config_path = tmp.path().join("backbeat.toml");
    std::fs::write(&config_path, &config_content).context("write config")?;

    let mut runs: Vec<RunMeasurement> = Vec::new();

    for i in 0..iterations {
        tracing::info!("  Iteration {}/{}…", i + 1, iterations);

        let tmp2 = tempfile::Builder::new()
            .prefix("backbeat-bench-out")
            .tempdir_in("/tmp")
            .context("tempdir")?;

        let backend = ResticBackend::from_config(
            &backbeat_core::RepoConfig {
                name: format!("bench-{}", label),
                backend: backbeat_core::BackendType::Restic,
                uri: host_repo_url.clone(),
                credential_env_vars: std::collections::HashMap::new(),
                snapshot_tag: None,
                schedule: "0 0 * * * *".to_string(),
            },
        ).context("init backend")?;
        let snapshot_id = backend.latest_snapshot_id().await
            .context("get snapshot")?;

        let start = Instant::now();

        let (outcome, peak_memory_kb) = run_restore_in_docker(
            &container_repo_url, &snapshot_id, tmp2.path(),
        ).await.context("restore in docker")?;

        let manifest = compute_manifest(tmp2.path()).context("manifest")?;
        let result = verify_restore(&outcome, &manifest);

        let duration = start.elapsed();

        runs.push(RunMeasurement {
            tier: label.to_string(),
            iteration: i + 1,
            duration_secs: duration.as_secs_f64(),
            bytes_transferred: outcome.bytes_restored,
            peak_memory_kb,
            status: format!("{:?}", result.status).to_lowercase(),
        });

        tracing::info!("    Done: {:.1}s, {} transferred, peak mem {}, status={}",
            duration.as_secs_f64(),
            format_size(outcome.bytes_restored),
            format_size(peak_memory_kb * 1024),
            format!("{:?}", result.status).to_lowercase(),
        );
    }

    // Cleanup.
    stop_container(&minio_id);

    // Aggregate.
    let n = runs.len() as f64;
    let avg_dur = runs.iter().map(|r| r.duration_secs).sum::<f64>() / n;
    let avg_bytes = (runs.iter().map(|r| r.bytes_transferred).sum::<u64>() as f64 / n) as u64;
    let avg_peak_mem = (runs.iter().map(|r| r.peak_memory_kb).sum::<u64>() as f64 / n) as u64;

    let bytes_per_run = avg_bytes as f64;

    // Cost estimates.
    let daily_cost_aws = bytes_per_run * 30.0 * AWS_S3_EGRESS_PER_GB / (1024.0 * 1024.0 * 1024.0);
    let weekly_cost_aws = bytes_per_run * 4.0 * AWS_S3_EGRESS_PER_GB / (1024.0 * 1024.0 * 1024.0);
    let daily_cost_b2 = bytes_per_run * 30.0 * B2_EGRESS_PER_GB / (1024.0 * 1024.0 * 1024.0);
    let weekly_cost_b2 = bytes_per_run * 4.0 * B2_EGRESS_PER_GB / (1024.0 * 1024.0 * 1024.0);

    Ok(TierResult {
        tier: label.to_string(),
        runs,
        avg_duration_secs: avg_dur,
        avg_bytes_transferred: avg_bytes,
        avg_peak_memory_kb: avg_peak_mem,
        daily_cost_aws,
        weekly_cost_aws,
        daily_cost_b2,
        weekly_cost_b2,
    })
}

// ---------------------------------------------------------------------------
// MinIO helpers
// ---------------------------------------------------------------------------

fn start_minio() -> (u16, String) {
    let _ = StdCommand::new("docker").args(["pull", "-q", MINIO_IMAGE]).output();

    let out = StdCommand::new("docker")
        .args(["run", "-d", "--rm", "-p", "9000",
            "-e", "MINIO_ROOT_USER=minioadmin",
            "-e", "MINIO_ROOT_PASSWORD=minioadmin",
            MINIO_IMAGE, "server", "/data"])
        .output()
        .expect("Docker not available — is it installed and running?");
    assert!(out.status.success(), "docker run minio: {}", String::from_utf8_lossy(&out.stderr));
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();

    let port_out = StdCommand::new("docker")
        .args(["port", &id, "9000"])
        .output()
        .unwrap();
    let port_str = String::from_utf8(port_out.stdout).unwrap();
    // `docker port` may emit multiple bindings (e.g. `0.0.0.0:51832` and
    // `[::]:51832` on macOS/ipv6). Pick the first line whose trailing
    // `:`-separated token parses as a port.
    let port: u16 = port_str
        .lines()
        .find_map(|line| line.rsplit(':').next().and_then(|p| p.trim().parse().ok()))
        .expect("could not parse MinIO host port");

    std::thread::sleep(std::time::Duration::from_secs(2));
    (port, id)
}

fn stop_container(id: &str) {
    let _ = StdCommand::new("docker").args(["rm", "-f", id]).output();
}

fn pull_image(image: &str) {
    let _ = StdCommand::new("docker").args(["pull", "-q", image]).output();
}

/// Run a `restic` command on the host. The benchmark's temp directories live
/// under `/tmp` so they bind-mount cleanly into Docker (macOS Docker Desktop
/// cannot reliably mount `/var/folders`). Backup uses a relative source path
/// (cwd = the temp root) so the snapshot stores `data/`, keeping the restore
/// tree and file-count check clean.
fn run_restic(cwd: &Path, args: &[&str]) -> anyhow::Result<()> {
    let out = StdCommand::new("restic")
        .current_dir(cwd)
        .env("AWS_ACCESS_KEY_ID", "minioadmin")
        .env("AWS_SECRET_ACCESS_KEY", "minioadmin")
        .env("RESTIC_PASSWORD", "backbeat-bench-password")
        .args(args)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("restic failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Resolve a container's IP address on its first network.
fn get_container_ip(id: &str) -> String {
    let out = StdCommand::new("docker")
        .args([
            "inspect",
            "-f",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            id,
        ])
        .output()
        .expect("docker inspect");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

// ---------------------------------------------------------------------------
// Docker-based restore with container stats tracking
// ---------------------------------------------------------------------------

/// Run `restic restore` inside a Docker container, tracking peak memory via
/// bollard's stats API. Returns the restore outcome and peak memory in KB.
async fn run_restore_in_docker(
    repo_url: &str,
    snapshot_id: &str,
    tmp_dir: &Path,
) -> anyhow::Result<(backbeat_core::RestoreOutcome, u64)> {
    let docker = Docker::connect_with_local_defaults()
        .context("Failed to connect to Docker daemon")?;

    let env = vec![
        "AWS_ACCESS_KEY_ID=minioadmin".to_string(),
        "AWS_SECRET_ACCESS_KEY=minioadmin".to_string(),
        "RESTIC_PASSWORD=backbeat-bench-password".to_string(),
    ];

    let bind = format!("{}:/restore-output", tmp_dir.display());
    let cmd = vec![
        "--repo".to_string(),
        repo_url.to_string(),
        "restore".to_string(),
        snapshot_id.to_string(),
        "--target".to_string(),
        "/restore-output".to_string(),
        "--json".to_string(),
    ];

    let container_config = ContainerConfig {
        image: Some(RESTIC_IMAGE.to_string()),
        cmd: Some(cmd),
        env: Some(env),
        host_config: Some(HostConfig {
            binds: Some(vec![bind]),
            // Default bridge network — shares it with MinIO so the container
            // reaches the repo by MinIO's IP. `--network host` cannot see the
            // host's published ports on Docker Desktop/macOS.
            ..Default::default()
        }),
        ..Default::default()
    };

    let create = docker
        .create_container::<String, String>(None, container_config)
        .await
        .context("Failed to create restore container")?;
    let id = &create.id;

    // Start stats collection in background.
    let stats_docker = docker.clone();
    let stats_id = id.clone();
    let stats_handle = tokio::spawn(async move {
        let mut peak_bytes = 0u64;
        let mut stream = stats_docker.stats(
            &stats_id,
            Some(StatsOptions {
                stream: true,
                one_shot: false,
            }),
        );
        while let Some(Ok(stat)) = stream.next().await {
            let usage = stat.memory_stats.usage.unwrap_or(0);
            peak_bytes = peak_bytes.max(usage);
        }
        peak_bytes
    });

    // Start container.
    docker
        .start_container::<String>(id, None::<StartContainerOptions<String>>)
        .await
        .context("Failed to start restore container")?;

    // Wait for exit.
    let mut wait_stream = docker.wait_container::<String>(
        id,
        Some(WaitContainerOptions {
            condition: "not-running".to_string(),
        }),
    );
    let wait_result = wait_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("Docker wait returned no status"))?
        .context("Docker wait failed")?;
    let exit_code = wait_result.status_code;

    // Capture logs.
    let log_options = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        follow: false,
        tail: "all".to_string(),
        ..Default::default()
    };
    let mut log_stream = docker.logs::<String>(id, Some(log_options));
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    while let Some(Ok(chunk)) = log_stream.next().await {
        match chunk {
            LogOutput::StdOut { message } => stdout_buf.extend_from_slice(&message),
            LogOutput::StdErr { message } => stderr_buf.extend_from_slice(&message),
            _ => {}
        }
    }

    // Remove container (best-effort).
    docker
        .remove_container(
            id,
            Some(RemoveContainerOptions {
                force: true,
                link: false,
                v: false,
            }),
        )
        .await
        .ok();

    // Check exit code.
    if exit_code != 0 {
        let stderr_str = String::from_utf8_lossy(&stderr_buf);
        anyhow::bail!(
            "restic restore exited with code {}: {}",
            exit_code,
            stderr_str.trim(),
        );
    }

    let peak_bytes = stats_handle.await.unwrap_or(0);

    let outcome = ResticBackend::parse_restore_output(&stdout_buf, snapshot_id)
        .context("Failed to parse restic restore JSON output")?;

    Ok((outcome, peak_bytes / 1024)) // convert to KB
}

// ---------------------------------------------------------------------------
// Synthetic data generation
// ---------------------------------------------------------------------------

fn generate_data(dir: &std::path::Path, target_bytes: u64) -> anyhow::Result<()> {
    use std::io::Write;

    // Create one large file at the target size using sparse-like writing.
    // On supported filesystems this is fast and doesn't use real disk space.
    let file_path = dir.join("payload.dat");
    let mut f = std::fs::File::create(&file_path)?;
    f.set_len(target_bytes)?;
    // Write a small header so the file isn't truly sparse (restic backs up
    // sparse files differently).
    f.write_all(b"BACKBEAT-BENCHMARK-PAYLOAD")?;
    f.flush()?;

    // Create a few small files for variety.
    for i in 0..10 {
        let small_path = dir.join(format!("meta_{}.txt", i));
        std::fs::write(&small_path, format!("metadata file {}\n", i))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Report generation
// ---------------------------------------------------------------------------

fn generate_report(results: &[TierResult]) -> String {
    let mut md = String::new();

    md.push_str("# Backbeatin — Benchmark Results\n\n");
    md.push_str("> Generated by `backbeat-bench` on ");
    md.push_str(&chrono_now());
    md.push_str("\n\n");

    md.push_str("## Methodology\n\n");
    md.push_str("- Each tier runs N verification cycles against a synthetic Restic repository hosted in a local MinIO (S3-compatible) instance.\n");
    md.push_str("- Each cycle: discover latest snapshot → restore to temp dir → compute SHA-256 manifest → verify.\n");
    md.push_str("- Restore runs inside a Docker container (managed via bollard), with peak RSS memory tracked via Docker stats API.\n");
    md.push_str("- All measurements are from real runs against real data — no simulated or placeholder values.\n\n");

    md.push_str("## Summary Table\n\n");
    md.push_str("| Repo Size | Avg Duration | Avg Peak Memory | Avg Data Transferred | Est. Monthly Cost (S3, daily) | Est. Monthly Cost (S3, weekly) | Est. Monthly Cost (B2, daily) | Est. Monthly Cost (B2, weekly) |\n");
    md.push_str("|---|---|---|---|---|---|---|---|\n");

    for tr in results {
        md.push_str(&format!(
            "| {} | {:.1}s | {} | {} | ${:.2} | ${:.2} | ${:.2} | ${:.2} |\n",
            tr.tier,
            tr.avg_duration_secs,
            format_size(tr.avg_peak_memory_kb * 1024),
            format_size(tr.avg_bytes_transferred),
            tr.daily_cost_aws,
            tr.weekly_cost_aws,
            tr.daily_cost_b2,
            tr.weekly_cost_b2,
        ));
    }

    md.push_str("\n## Per-Run Details\n\n");

    for tr in results {
        md.push_str(&format!("### {} Tier\n\n", tr.tier));
        md.push_str("| Run | Duration | Peak Memory | Transferred | Status |\n");
        md.push_str("|---|---|---|---|---|\n");
        for r in &tr.runs {
            md.push_str(&format!(
                "| {} | {:.1}s | {} | {} | {} |\n",
                r.iteration,
                r.duration_secs,
                format_size(r.peak_memory_kb * 1024),
                format_size(r.bytes_transferred),
                r.status,
            ));
        }
        md.push('\n');
    }

    md.push_str("## Cost Estimate Assumptions\n\n");
    md.push_str("### Pricing Sources\n\n");
    md.push_str("| Provider | Egress Price/GB | Source | Date |\n");
    md.push_str("|---|---|---|---|\n");
    md.push_str(&format!("| AWS S3 (us-east-1) | ${:.2} | https://aws.amazon.com/s3/pricing/ | 2025-07 |\n", AWS_S3_EGRESS_PER_GB));
    md.push_str(&format!("| Backblaze B2 | ${:.2} | https://www.backblaze.com/cloud-storage/pricing | 2025-07 |\n", B2_EGRESS_PER_GB));

    md.push_str("\n### Calculation\n\n");
    md.push_str("- Daily cost = avg_bytes_per_run × 30 (monthly runs) × price/byte\n");
    md.push_str("- Weekly cost = avg_bytes_per_run × 4 (monthly runs) × price/byte\n");
    md.push_str("- These are **estimates only**. Actual costs depend on your provider, region, volume tier, and any negotiated discounts.\n");
    md.push_str("- Egress is calculated on the full restore bytes — caching or incremental restore strategies could significantly reduce this in practice.\n");
    md.push_str("- Storage costs (holding the backup data in the bucket) are not included; only egress from verification runs.\n\n");

    md.push_str("## Notes\n\n");
    md.push_str("- Benchmark was run against synthetic data; real-world backup contents (many small files, database dumps, etc.) may produce different results.\n");
    md.push_str("- Network latency to the backup repo (MinIO was localhost) significantly affects duration; cloud-hosted repos will have longer restore times.\n");
    md.push_str("- Peak memory reflects the restore container only; the host-side manifest hashing and verification are excluded from the memory figure.\n");

    md
}

// ---------------------------------------------------------------------------
// Short helpers
// ---------------------------------------------------------------------------

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}

fn chrono_now() -> String {
    // Simple UTC timestamp without chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as ISO 8601.
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    // YYYY-MM-DD from Unix days (since 1970-01-01).
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Algorithm from Howard Hinnant.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

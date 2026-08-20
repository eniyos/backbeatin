use std::process::Command as StdCommand;
use std::time::Duration;

const BACKBEAT_BIN: &str = env!("CARGO_BIN_EXE_backbeat");
const MINIO_IMAGE: &str = "minio/minio:latest";
const RESTIC_IMAGE: &str = "restic/restic:latest";

/// Start a MinIO container and return (host_port, container_id).
fn start_minio() -> (u16, String) {
    // Pull images first (best-effort).
    let _ = StdCommand::new("docker")
        .args(["pull", "-q", MINIO_IMAGE])
        .output();
    let _ = StdCommand::new("docker")
        .args(["pull", "-q", RESTIC_IMAGE])
        .output();

    // Start MinIO on a random port.
    let output = StdCommand::new("docker")
        .args([
            "run", "-d", "--rm",
            "-p", "9000",
            "-e", "MINIO_ROOT_USER=minioadmin",
            "-e", "MINIO_ROOT_PASSWORD=minioadmin",
            MINIO_IMAGE, "server", "/data",
        ])
        .output()
        .expect("Failed to run Docker — is Docker installed and running?");
    assert!(output.status.success(), "docker run minio failed");

    let container_id = String::from_utf8(output.stdout)
        .expect("container ID")
        .trim()
        .to_string();

    // Read the mapped host port.
    let port_output = StdCommand::new("docker")
        .args(["port", &container_id, "9000"])
        .output()
        .expect("docker port");
    let port_str = String::from_utf8(port_output.stdout)
        .expect("port output");
    // `docker port` may emit multiple bindings (e.g. `0.0.0.0:51832` and
    // `[::]:51832` on macOS/ipv6). Pick the first line whose trailing
    // `:`-separated token parses as a port.
    let host_port: u16 = port_str
        .lines()
        .find_map(|line| line.rsplit(':').next().and_then(|p| p.trim().parse().ok()))
        .expect("could not parse MinIO host port");

    std::thread::sleep(Duration::from_secs(2));
    (host_port, container_id)
}

/// Stop and remove a container.
fn stop_container(id: &str) {
    let _ = StdCommand::new("docker")
        .args(["rm", "-f", id])
        .output();
}

/// Run restic via Docker with the given args.
fn restic(_host_port: u16, args: &[&str]) -> std::process::Output {
    StdCommand::new("docker")
        .args([
            "run", "--rm",
            "--network", "host",
            "-e", "AWS_ACCESS_KEY_ID=minioadmin",
            "-e", "AWS_SECRET_ACCESS_KEY=minioadmin",
            RESTIC_IMAGE,
        ])
        .args(args)
        .output()
        .expect("restic command failed")
}

#[test]
fn test_restic_s3_verify_pass() {
    let (port, minio_id) = start_minio();
    let repo_url = format!("s3:http://127.0.0.1:{}/testrepo", port);

    // 1. Init repo.
    let out = restic(port, &["init", "--repo", &repo_url]);
    assert!(out.status.success(), "restic init: {}", String::from_utf8_lossy(&out.stderr));

    // 2. Create test files.
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create dir");
    std::fs::write(data_dir.join("f1.txt"), b"hello").expect("write f1");
    std::fs::write(data_dir.join("f2.txt"), b"world").expect("write f2");

    // 3. Backup.
    let out = restic(port, &["backup", "--repo", &repo_url, data_dir.to_str().unwrap()]);
    assert!(out.status.success(), "restic backup: {}", String::from_utf8_lossy(&out.stderr));

    // 4. Run backbeat verify.
    let config = format!(
        "[[repo]]\nname = \"test-s3\"\nbackend = \"restic\"\nuri = \"{}\"\n[repo.credential_env_vars]\nAWS_ACCESS_KEY_ID = \"x\"\nAWS_SECRET_ACCESS_KEY = \"x\"\n",
        repo_url,
    );
    let config_path = tmp.path().join("backbeat.toml");
    std::fs::write(&config_path, &config).expect("write config");
    let db_path = tmp.path().join("backbeat.db");

    let out = StdCommand::new(BACKBEAT_BIN)
        .args(["verify", "test-s3", "--config", config_path.to_str().unwrap(), "--db-path", db_path.to_str().unwrap()])
        .env("AWS_ACCESS_KEY_ID", "minioadmin")
        .env("AWS_SECRET_ACCESS_KEY", "minioadmin")
        .output()
        .expect("backbeat verify");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    stop_container(&minio_id);

    assert!(out.status.success(), "backbeat verify FAILED (expected pass)\nstdout: {}\nstderr: {}", stdout, stderr);
    assert!(stdout.contains("verified"), "Expected verified message, got: {}", stdout);
}

#[test]
fn test_restic_s3_verify_fail_on_corrupt() {
    let (port, minio_id) = start_minio();
    let repo_url = format!("s3:http://127.0.0.1:{}/testrepo", port);

    // 1. Init + backup.
    let out = restic(port, &["init", "--repo", &repo_url]);
    assert!(out.status.success(), "init: {}", String::from_utf8_lossy(&out.stderr));

    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create dir");
    std::fs::write(data_dir.join("f.txt"), b"data").expect("write");
    let out = restic(port, &["backup", "--repo", &repo_url, data_dir.to_str().unwrap()]);
    assert!(out.status.success(), "backup: {}", String::from_utf8_lossy(&out.stderr));

    // 2. Corrupt the repo (remove the restic config from MinIO storage).
    // MinIO stores data at /data/<bucket>/... — deleting the config
    // makes the repo unrecognisable to restic.
    let _ = StdCommand::new("docker")
        .args([
            "exec", &minio_id,
            "sh", "-c", "rm -rf /data/testrepo/config",
        ])
        .output();

    // 3. Run verify — should fail.
    let config = format!(
        "[[repo]]\nname = \"test-s3\"\nbackend = \"restic\"\nuri = \"{}\"\n[repo.credential_env_vars]\nAWS_ACCESS_KEY_ID = \"x\"\nAWS_SECRET_ACCESS_KEY = \"x\"\n",
        repo_url,
    );
    let config_path = tmp.path().join("backbeat.toml");
    std::fs::write(&config_path, &config).expect("write config");
    let db_path = tmp.path().join("backbeat.db");

    let out = StdCommand::new(BACKBEAT_BIN)
        .args(["verify", "test-s3", "--config", config_path.to_str().unwrap(), "--db-path", db_path.to_str().unwrap()])
        .env("AWS_ACCESS_KEY_ID", "minioadmin")
        .env("AWS_SECRET_ACCESS_KEY", "minioadmin")
        .output()
        .expect("backbeat verify");

    let stderr = String::from_utf8_lossy(&out.stderr);
    stop_container(&minio_id);

    assert!(!out.status.success(), "backbeat verify PASSED (should have failed on corrupt repo)\nstderr: {}", stderr);
}

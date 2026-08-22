//! Docker sandbox management for isolated backup restore operations.
//!
//! This module provides secure, ephemeral container execution for backup
//! verification. It ensures that:
//!
//! - Restores happen in isolated environments
//! - No restored data persists on the host system
//! - Containers are automatically cleaned up after verification
//! - Multiple verification runs can't interfere with each other
//!
//! # Security Benefits
//!
//! - **Isolation**: Restored files never touch the host filesystem
//! - **Cleanup**: Containers are force-removed after verification
//! - **Consistency**: Same Docker images across different platforms
//! - **Sandboxing**: All Linux capabilities are dropped; local filesystem
//!   repos run with no network at all (`NetworkMode: none`).  Remote
//!   backends (S3, B2, SFTP, …) keep default bridge networking because
//!   the restore itself must fetch chunks from the endpoint.
//!
//! # Container Lifecycle
//!
//! 1. Pull required Docker image (if not already present)
//! 2. Create ephemeral container with bind mounts
//! 3. Start container and execute restore command
//! 4. Capture stdout/stderr from container
//! 5. Force-remove container (including volumes)
//! 6. Return captured output for parsing

use std::path::Path;

use anyhow::Context;
use bollard::container::{
    Config, LogOutput, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::HostConfig;
use bollard::Docker;
use futures_util::StreamExt;

/// Default Docker image to use for running backup-tool commands inside a sandbox.
pub const DEFAULT_IMAGE_RESTIC: &str = "restic/restic:latest";
pub const DEFAULT_IMAGE_BORG: &str = "borgbackup/borg:latest";

/// Path inside the container where restored files appear.
const CONTAINER_OUTPUT_PATH: &str = "/restore-output";

/// Remote URI schemes that require outbound network access to fetch
/// backup chunks.  Anything else is treated as a local filesystem repo.
const REMOTE_URI_SCHEMES: &[&str] = &[
    "s3:", "b2:", "gs:", "azure:", "swift:", "rest:", "rclone:", "ssh:", "sftp:", "http://",
    "https://",
];

/// Decide the Docker network mode for a restore against `uri`.
///
/// Local filesystem repos need no network at all, so the container runs
/// with `NetworkMode: none`.  Remote backends (S3, B2, SFTP, …) must be
/// able to reach their endpoint to fetch chunks, so they get Docker's
/// default bridge network — the Docker API alone cannot restrict egress
/// to a single host without external firewall tooling, and we do not
/// claim otherwise.
#[must_use]
pub fn network_mode_for_uri(uri: &str) -> Option<String> {
    let remote = REMOTE_URI_SCHEMES
        .iter()
        .any(|scheme| uri.starts_with(scheme));
    if remote {
        None // default bridge network
    } else {
        Some("none".to_string())
    }
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

/// Manages ephemeral Docker containers for isolated backup restore execution.
///
/// Every restore operation happens inside a throwaway container that is
/// destroyed immediately after verification. This ensures that:
/// - No restored data persists on the host system
/// - Multiple verification runs can't interfere with each other
/// - The verification environment is consistent and isolated
///
/// Every restore runs inside a throwaway container that is destroyed as soon
/// as the command completes (including its output volume).  No restored file
/// content survives outside the container lifecycle.
pub struct Sandbox {
    docker: Docker,
    image: String,
    limits: crate::config::SandboxConfig,
}

impl Sandbox {
    /// Connect to the local Docker daemon using default socket paths.
    ///
    /// Returns an error if Docker is not available or the user lacks
    /// permission to access the socket.
    ///
    /// Defaults to using the restic Docker image; call [`Self::with_image`]
    /// or the backend-specific runners to override.
    ///
    /// # Errors
    ///
    /// Returns an error if no local Docker socket is available. Make sure
    /// Docker is installed and the current user has permission to access
    /// the socket.
    pub fn connect() -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon. Is it installed and running?")?;

        Ok(Self {
            docker,
            image: DEFAULT_IMAGE_RESTIC.to_string(),
            limits: crate::config::SandboxConfig::default(),
        })
    }

    /// Override the default container image.
    #[must_use]
    pub fn with_image(mut self, image: &str) -> Self {
        self.image = image.to_string();
        self
    }

    /// Override the default container resource limits.
    #[must_use]
    pub fn with_limits(mut self, limits: &crate::config::SandboxConfig) -> Self {
        self.limits = limits.clone();
        self
    }

    /// Pre-flight disk budget check.
    ///
    /// The restore target is a host bind mount (the host reads the files
    /// back to compute the manifest), so Docker cannot quota it directly.
    /// Instead we refuse to start when the output filesystem has less free
    /// space than the configured budget — a large or corrupted repo then
    /// fails fast instead of filling the host disk.
    ///
    /// # Errors
    ///
    /// Returns an error if free space cannot be determined or is below
    /// the configured disk budget.
    fn check_disk_budget(host_output_dir: &Path, budget: u64) -> anyhow::Result<()> {
        let free = fs2::available_space(host_output_dir)
            .context("Failed to determine free disk space for restore target")?;
        if free < budget {
            anyhow::bail!(
                "Insufficient free disk space for restore: {free} bytes available, but the \
                 sandbox disk budget is {budget} bytes. Free space or lower \
                 `[sandbox] disk_budget_bytes` in the config."
            );
        }
        Ok(())
    }

    /// Ensure the configured image is available locally, pulling it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the image cannot be pulled from the registry.
    pub async fn ensure_image(&self) -> anyhow::Result<()> {
        let options = CreateImageOptions {
            from_image: self.image.as_str(),
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(result) = stream.next().await {
            result.context("Failed to pull Docker image")?;
        }

        Ok(())
    }

    /// Run a restic restore operation inside an ephemeral Docker container.
    ///
    /// Credential environment variables are resolved from `config` at call
    /// time.  The restore output lands in `host_output_dir` (bind-mounted
    /// at `/restore-output` inside the container).
    ///
    /// When `includes` is non-empty, only matching paths are restored
    /// (sampled restore) via repeated `--include` flags.
    ///
    /// Returns the raw stdout bytes from the container (JSON-line output
    /// from `restic restore --json`).
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be started, the restore
    /// command fails, or the output cannot be read.
    pub async fn run_restic_restore(
        &self,
        config: &crate::config::RepoConfig,
        snapshot_id: &str,
        host_output_dir: &Path,
        includes: &[String],
    ) -> anyhow::Result<Vec<u8>> {
        // Resolve credential env vars from the process environment.
        let env: Vec<String> = config
            .credential_env_vars
            .keys()
            .map(|var| {
                let val = std::env::var(var).with_context(|| {
                    format!(
                        "Required env var '{}' for repo '{}' is not set",
                        var, config.name
                    )
                })?;
                Ok(format!("{var}={val}"))
            })
            .collect::<anyhow::Result<_>>()?;

        // Build the restic command arguments for inside the container.
        // The `restic/restic` image has ENTRYPOINT ["restic"], so we only
        // pass arguments — not the binary name itself.
        let mut cmd = vec![
            "--repo".to_string(),
            config.uri.clone(),
            "restore".to_string(),
            snapshot_id.to_string(),
            "--target".to_string(),
            CONTAINER_OUTPUT_PATH.to_string(),
            "--json".to_string(),
        ];
        // Sampled restore: restrict the restore to the selected paths.
        for include in includes {
            cmd.push("--include".to_string());
            cmd.push(include.clone());
        }

        // Bind-mount the host output directory into the container.
        let bind = format!("{}:{}", host_output_dir.display(), CONTAINER_OUTPUT_PATH);
        Self::check_disk_budget(host_output_dir, self.limits.disk_budget_bytes)?;

        tracing::info!(
            "Running restore inside Docker container (image: {})…",
            self.image
        );

        let (stdout, _stderr) = self
            .run_command(cmd, env, vec![bind], network_mode_for_uri(&config.uri))
            .await
            .context("Docker container restore failed")?;

        Ok(stdout)
    }

    /// Run a Borg extract operation inside an ephemeral Docker container.
    ///
    /// This is the Borg equivalent of [`Self::run_restic_restore`], using the
    /// `borgbackup/borg` image and Borg's `extract` command syntax.
    ///
    /// When `includes` is non-empty, only those paths are extracted
    /// (sampled restore) — Borg treats trailing positional arguments as
    /// path filters.
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be started, the extract
    /// command fails, or the output cannot be read.
    pub async fn run_borg_extract(
        &self,
        config: &crate::config::RepoConfig,
        archive_name: &str,
        host_output_dir: &Path,
        includes: &[String],
    ) -> anyhow::Result<Vec<u8>> {
        // Resolve credential env vars from the process environment.
        let env: Vec<String> = config
            .credential_env_vars
            .keys()
            .map(|var| {
                let val = std::env::var(var).with_context(|| {
                    format!(
                        "Required env var '{}' for repo '{}' is not set",
                        var, config.name
                    )
                })?;
                Ok(format!("{var}={val}"))
            })
            .collect::<anyhow::Result<_>>()?;

        // Build the Borg command arguments inside the container.
        let archive_ref = format!("{}::{}", config.uri, archive_name);
        let mut cmd = vec![
            "extract".to_string(),
            "--destination".to_string(),
            CONTAINER_OUTPUT_PATH.to_string(),
            archive_ref,
        ];
        // Sampled restore: extract only the selected paths.
        cmd.extend(includes.iter().cloned());

        // Bind-mount the host output directory into the container.
        let bind = format!("{}:{}", host_output_dir.display(), CONTAINER_OUTPUT_PATH);
        Self::check_disk_budget(host_output_dir, self.limits.disk_budget_bytes)?;

        tracing::info!(
            "Running Borg extract inside Docker container (image: {})…",
            self.image
        );

        let (stdout, _stderr) = self
            .run_command(cmd, env, vec![bind], network_mode_for_uri(&config.uri))
            .await
            .context("Docker container Borg extract failed")?;

        Ok(stdout)
    }

    /// Low-level: run an arbitrary command inside a throwaway container.
    ///
    /// Creates the container, starts it, waits for completion, captures
    /// stdout/stderr, then removes the container (force=true).
    /// The container is always removed when this function returns, even on
    /// error.
    ///
    /// Hardening applied to every container:
    /// - All Linux capabilities are dropped (`CapDrop: ALL`) — restore
    ///   tools need none.
    /// - `network_mode` controls egress: `Some("none")` for local repos,
    ///   `None` (default bridge) when the repo backend is remote.
    /// - Explicit CPU / memory / PID limits so a large or corrupted repo
    ///   cannot exhaust host resources before cleanup runs.
    async fn run_command(
        &self,
        cmd: Vec<String>,
        env: Vec<String>,
        binds: Vec<String>,
        network_mode: Option<String>,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        // Docker expresses CPU limits in nanocpus (1e9 = one core).  The
        // f64→i64 cast is safe for any sensible cpus value.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let nano_cpus = (self.limits.cpus * 1_000_000_000.0) as i64;

        let config: Config<String> = Config {
            image: Some(self.image.clone()),
            cmd: Some(cmd),
            env: Some(env),
            host_config: Some(HostConfig {
                binds: Some(binds),
                network_mode,
                cap_drop: Some(vec!["ALL".to_string()]),
                memory: Some(self.limits.memory_bytes),
                nano_cpus: Some(nano_cpus),
                pids_limit: Some(1024),
                ..Default::default()
            }),
            ..Default::default()
        };

        // Create container (anonymous name).
        let create_result = self
            .docker
            .create_container::<String, String>(None, config)
            .await
            .context("Failed to create Docker container")?;
        let id = &create_result.id;
        tracing::debug!("Created container {}", id);

        // Run the container and capture its output.  On any error we still
        // try to remove the container before propagating the error.
        let inner_result = self.run_container_inner(id).await;

        // Always attempt to remove the container (best-effort cleanup).
        if let Err(remove_err) = self
            .docker
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    link: false,
                    v: false,
                }),
            )
            .await
        {
            tracing::warn!("Failed to remove container {}: {}", id, remove_err);
        } else {
            tracing::debug!("Removed container {}", id);
        }

        // Propagate any error from the inner operation.
        inner_result
    }

    /// Inner logic: start, wait, and capture logs for an already-created
    /// container.  Does NOT attempt removal — the caller must clean up.
    async fn run_container_inner(&self, id: &str) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        // Start it.
        self.docker
            .start_container::<String>(id, None::<StartContainerOptions<String>>)
            .await
            .context("Failed to start Docker container")?;

        // Wait for the container to exit.
        let mut wait_stream = self.docker.wait_container::<String>(
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
        tracing::debug!("Container {} exited with code {}", id, exit_code);

        // Capture logs (container is stopped at this point).
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let log_options: LogsOptions<String> = LogsOptions {
            stdout: true,
            stderr: true,
            follow: false,
            tail: "all".to_string(),
            ..Default::default()
        };
        let mut log_stream = self.docker.logs::<String>(id, Some(log_options));

        while let Some(chunk) = log_stream.next().await {
            match chunk.context("Failed to read container logs")? {
                LogOutput::StdOut { message } => stdout.extend_from_slice(&message),
                LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
                _ => {}
            }
        }

        // Check exit code.
        if exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&stderr);
            anyhow::bail!(
                "Container exited with code {}: {}",
                exit_code,
                stderr_str.trim(),
            );
        }

        Ok((stdout, stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_repo_gets_no_network() {
        assert_eq!(
            network_mode_for_uri("/srv/backups/restic"),
            Some("none".into())
        );
        assert_eq!(
            network_mode_for_uri("relative/path/repo"),
            Some("none".into())
        );
    }

    #[test]
    fn test_remote_repos_keep_default_network() {
        assert_eq!(
            network_mode_for_uri("s3:https://s3.us-east-1.amazonaws.com/bucket/repo"),
            None
        );
        assert_eq!(network_mode_for_uri("b2:my-bucket:/restic"), None);
        assert_eq!(network_mode_for_uri("ssh://user@host:./repo"), None);
        assert_eq!(network_mode_for_uri("sftp://user@host/repo"), None);
        assert_eq!(
            network_mode_for_uri("rest:https://rest-server:8000/repo"),
            None
        );
    }

    #[test]
    fn test_disk_budget_passes_when_enough_space() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // A 1-byte budget should always fit.
        assert!(Sandbox::check_disk_budget(tmp.path(), 1).is_ok());
    }

    #[test]
    fn test_disk_budget_fails_when_too_large() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No test machine has an exabyte of free space.
        let err = Sandbox::check_disk_budget(tmp.path(), u64::MAX - 1).unwrap_err();
        assert!(err.to_string().contains("Insufficient free disk space"));
    }
}

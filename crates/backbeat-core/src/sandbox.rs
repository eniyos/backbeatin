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

/// Default Docker image to use for running restic commands inside a sandbox.
const DEFAULT_IMAGE: &str = "restic/restic:latest";

/// Path inside the container where restored files appear.
const CONTAINER_OUTPUT_PATH: &str = "/restore-output";

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

/// Manages ephemeral Docker containers for isolated backup restore execution.
///
/// Every restore runs inside a throwaway container that is destroyed as soon
/// as the command completes (including its output volume).  No restored file
/// content survives outside the container lifecycle.
pub struct Sandbox {
    docker: Docker,
    image: String,
}

impl Sandbox {
    /// Connect to the local Docker daemon using default socket paths.
    ///
    /// Returns an error if Docker is not available or the user lacks
    /// permission to access the socket.
    pub async fn connect() -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon. Is it installed and running?")?;

        Ok(Self {
            docker,
            image: DEFAULT_IMAGE.to_string(),
        })
    }

    /// Override the default container image.
    #[allow(dead_code)]
    pub fn with_image(mut self, image: &str) -> Self {
        self.image = image.to_string();
        self
    }

    /// Ensure the configured image is available locally, pulling it if needed.
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
    /// Returns the raw stdout bytes from the container (JSON-line output
    /// from `restic restore --json`).
    pub async fn run_restic_restore(
        &self,
        config: &crate::config::RepoConfig,
        snapshot_id: &str,
        host_output_dir: &Path,
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
                Ok(format!("{}={}", var, val))
            })
            .collect::<anyhow::Result<_>>()?;

        // Build the restic command line inside the container.
        let cmd = vec![
            "restic".to_string(),
            "--repo".to_string(),
            config.uri.clone(),
            "restore".to_string(),
            snapshot_id.to_string(),
            "--target".to_string(),
            CONTAINER_OUTPUT_PATH.to_string(),
            "--json".to_string(),
        ];

        // Bind-mount the host output directory into the container.
        let bind = format!("{}:{}", host_output_dir.display(), CONTAINER_OUTPUT_PATH);

        tracing::info!(
            "Running restore inside Docker container (image: {})…",
            self.image
        );

        let (stdout, _stderr) = self
            .run_command(cmd, env, vec![bind])
            .await
            .context("Docker container restore failed")?;

        Ok(stdout)
    }

    /// Low-level: run an arbitrary command inside a throwaway container.
    ///
    /// Creates the container, starts it, waits for completion, captures
    /// stdout/stderr, then removes the container (force=true).
    async fn run_command(
        &self,
        cmd: Vec<String>,
        env: Vec<String>,
        binds: Vec<String>,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let config: Config<String> = Config {
            image: Some(self.image.clone()),
            cmd: Some(cmd),
            env: Some(env),
            host_config: Some(HostConfig {
                binds: Some(binds),
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

        // Capture logs.
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

        // Remove container (force).
        self.docker
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force: true,
                    link: false,
                    v: false,
                }),
            )
            .await
            .context("Failed to remove Docker container")?;

        tracing::debug!("Removed container {}", id);

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

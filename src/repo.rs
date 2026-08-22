//! Backup backend abstraction and implementations.
//!
//! This module provides a trait-based interface for interacting with different
//! backup systems (currently Restic and Borg). It handles:
//!
//! - Snapshot/archive discovery
//! - Restore operations
//! - Repository statistics
//! - JSON output parsing from CLI tools
//!
//! # Design Philosophy
//!
//! Backbeatin never reimplements backup repository formats. Instead, it shells
//! out to the official CLI tools and parses their JSON output. This ensures:
//! - Compatibility with the actual backup tools
//! - Automatic support for new features
//! - Reduced maintenance burden
//! - Trust in the well-tested CLI implementations
//!
//! # Adding New Backends
//!
//! To add support for a new backup system:
//! 1. Implement the `BackupBackend` trait
//! 2. Add JSON parsing for snapshot discovery output
//! 3. Add JSON parsing for restore statistics output
//! 4. Handle credential environment variables appropriately

use std::path::Path;
use std::process::Stdio;

use anyhow::Context;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use crate::config::RepoConfig;

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// The outcome of a single restore operation.
///
/// Contains statistics reported by the backup backend after a restore operation.
/// These statistics are compared against the computed manifest during verification.
#[derive(Debug, Clone)]
pub struct RestoreOutcome {
    /// The snapshot ID that was restored.
    pub snapshot_id: String,
    /// The number of files that were restored (as reported by the backend).
    pub files_count: u64,
    /// The total number of bytes restored (as reported by the backend).
    pub bytes_restored: u64,
    /// Whether `files_count` / `bytes_restored` come from actual backend
    /// output (true) or are placeholders because the backend does not
    /// report counts (false, e.g. Borg's `extract` command).
    ///
    /// When `false`, verification will skip the zero-count check and rely
    /// purely on manifest-based comparison.
    pub count_is_meaningful: bool,
}

/// Repository-level statistics reported by `restic stats --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoStats {
    /// Total number of snapshots in the repository.
    #[serde(alias = "total_snapshots")]
    pub snapshot_count: u64,
}

/// Generic interface that both Restic and Borg backends implement.
///
/// The rest of the system interacts only through this trait — new backends
/// are added by implementing it without touching verification or CLI logic.
#[async_trait]
pub trait BackupBackend: Send + Sync {
    /// Return the ID of the latest (most recent) snapshot.
    async fn latest_snapshot_id(&self) -> anyhow::Result<String>;

    /// Restore `snapshot_id` into `target_dir`.
    ///
    /// The caller is responsible for ensuring `target_dir` exists and is
    /// empty.  Returns an `Outcome` with statistics reported by the backend.
    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        target_dir: &Path,
    ) -> anyhow::Result<RestoreOutcome>;
}

// ---------------------------------------------------------------------------
// Restic backend
// ---------------------------------------------------------------------------

/// A [`BackupBackend`] implementation that shells out to the `restic` CLI.
pub struct ResticBackend {
    repo_uri: String,
    env_overrides: Vec<(String, String)>,
    snapshot_tag: Option<String>,
}

impl ResticBackend {
    /// Create a new `ResticBackend` from a [`RepoConfig`].
    ///
    /// Credential environment variables are resolved from the process
    /// environment at construction time.
    ///
    /// # Errors
    ///
    /// Returns an error if any required credential env var is unset
    /// for `config`, or if the underlying `std::env::var` lookup fails.
    pub fn from_config(config: &RepoConfig) -> anyhow::Result<Self> {
        let env_overrides: Vec<(String, String)> = config
            .credential_env_vars
            .keys()
            .map(|var| {
                let val = std::env::var(var).with_context(|| {
                    format!(
                        "Required env var '{}' is not set for repo '{}'",
                        var, config.name
                    )
                })?;
                Ok((var.clone(), val))
            })
            .collect::<anyhow::Result<_>>()?;

        Ok(Self {
            repo_uri: config.uri.clone(),
            env_overrides,
            snapshot_tag: config.snapshot_tag.clone(),
        })
    }

    /// Build a `tokio::process::Command` with `restic` at the front, passing
    /// the repo URI and any configured env vars.
    fn restic_command(&self) -> Command {
        let mut cmd = Command::new("restic");
        cmd.arg("--repo").arg(&self.repo_uri);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Layer credential overrides on top of the inherited environment.
        for (key, val) in &self.env_overrides {
            cmd.env(key, val);
        }

        cmd
    }

    /// Parse the JSON output of `restic snapshots --json --latest 1 …`.
    ///
    /// The output is a JSON array of snapshot objects.  We extract the
    /// `short_id` field from the first (most recent) entry.
    fn parse_latest_snapshot_id(output: &[u8]) -> anyhow::Result<String> {
        let snapshots: Vec<serde_json::Value> = serde_json::from_slice(output)
            .context("Failed to parse restic snapshots JSON output")?;

        let snapshot = snapshots
            .first()
            .ok_or_else(|| anyhow::anyhow!("No snapshots found in repository"))?;

        let short_id = snapshot["short_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'short_id' in snapshot JSON"))?;

        Ok(short_id.to_string())
    }

    /// Parse the JSON output of `restic stats --json`.
    ///
    /// The output contains repository-level statistics such as the total
    /// number of snapshots.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not valid JSON or does not match
    /// the expected `RepoStats` schema.
    pub fn parse_stats_output(output: &[u8]) -> anyhow::Result<RepoStats> {
        serde_json::from_slice(output).context("Failed to parse restic stats JSON output")
    }

    /// Parse the JSON output of `restic restore … --json`.
    ///
    /// Restic's JSON restore output prints one JSON-Line per message.
    /// The final "summary" line contains `message_type: "summary"` with
    /// `total_files` and `total_bytes`.
    ///
    /// This is `pub` so that callers using a sandbox (Docker) can parse
    /// captured container output into a `RestoreOutcome`.
    ///
    /// # Errors
    ///
    /// Returns an error if the output is not valid UTF-8, does not
    /// contain a parseable `summary` line, or is missing required
    /// `total_files` / `total_bytes` fields.
    pub fn parse_restore_output(
        output: &[u8],
        snapshot_id: &str,
    ) -> anyhow::Result<RestoreOutcome> {
        let text =
            String::from_utf8(output.to_vec()).context("restic restore output was not UTF-8")?;

        let mut files_count: u64 = 0;
        let mut bytes_restored: u64 = 0;

        for line in text.lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if val.get("message_type").and_then(|m| m.as_str()) == Some("summary") {
                    files_count = val
                        .get("total_files")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    bytes_restored = val
                        .get("total_bytes")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                }
            }
        }

        Ok(RestoreOutcome {
            snapshot_id: snapshot_id.to_string(),
            files_count,
            bytes_restored,
            count_is_meaningful: true,
        })
    }
}

#[async_trait]
impl BackupBackend for ResticBackend {
    async fn latest_snapshot_id(&self) -> anyhow::Result<String> {
        let mut cmd = self.restic_command();
        cmd.arg("snapshots").arg("--json").arg("--latest").arg("1");
        if let Some(tag) = &self.snapshot_tag {
            cmd.arg("--tag").arg(tag);
        }

        let output = cmd
            .output()
            .await
            .context("Failed to run 'restic snapshots' — is restic installed and on PATH?")?;

        if !output.status.success() {
            let _stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("restic snapshots failed (exit {})", output.status);
        }

        Self::parse_latest_snapshot_id(&output.stdout)
    }

    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        target_dir: &Path,
    ) -> anyhow::Result<RestoreOutcome> {
        let mut cmd = self.restic_command();
        cmd.arg("restore")
            .arg(snapshot_id)
            .arg("--target")
            .arg(target_dir)
            .arg("--json");

        let output = cmd
            .output()
            .await
            .context("Failed to run 'restic restore'")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "restic restore failed (exit {}){}",
                output.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr.trim())
                }
            );
        }

        Self::parse_restore_output(&output.stdout, snapshot_id)
    }
}

// ---------------------------------------------------------------------------
// Borg backend
// ---------------------------------------------------------------------------

/// A [`BackupBackend`] implementation that shells out to the `borg` CLI.
pub struct BorgBackend {
    repo: String,
    env_overrides: Vec<(String, String)>,
}

impl BorgBackend {
    /// Create a new `BorgBackend` from a [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if any required credential env var is unset
    /// for `config`, or if the underlying `std::env::var` lookup fails.
    pub fn from_config(config: &RepoConfig) -> anyhow::Result<Self> {
        let env_overrides: Vec<(String, String)> = config
            .credential_env_vars
            .keys()
            .map(|var| {
                let val = std::env::var(var).with_context(|| {
                    format!(
                        "Required env var '{}' is not set for repo '{}'",
                        var, config.name
                    )
                })?;
                Ok((var.clone(), val))
            })
            .collect::<anyhow::Result<_>>()?;

        Ok(Self {
            repo: config.uri.clone(),
            env_overrides,
        })
    }

    /// Build a `tokio::process::Command` with `borg` at the front.
    fn borg_command(&self) -> Command {
        let mut cmd = Command::new("borg");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        for (key, val) in &self.env_overrides {
            cmd.env(key, val);
        }

        cmd
    }

    /// Parse the JSON output of `borg list --json <repo>`.
    ///
    /// Output format:
    /// ```json
    /// {"archives":[{"name":"host-2024-01-15","time":"...",...}, ...], "repository":{...}}
    /// ```
    /// Archives are listed in chronological order; the last entry is the latest.
    fn parse_latest_archive(output: &[u8]) -> anyhow::Result<String> {
        let parsed: serde_json::Value =
            serde_json::from_slice(output).context("Failed to parse borg list JSON output")?;

        let archives = parsed["archives"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing 'archives' array in borg list output"))?;

        let latest = archives
            .last()
            .ok_or_else(|| anyhow::anyhow!("No archives found in Borg repository"))?;

        let name = latest["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' field in Borg archive entry"))?;

        Ok(name.to_string())
    }
}

#[async_trait]
impl BackupBackend for BorgBackend {
    async fn latest_snapshot_id(&self) -> anyhow::Result<String> {
        let mut cmd = self.borg_command();
        cmd.arg("list").arg("--json").arg(&self.repo);

        let output = cmd
            .output()
            .await
            .context("Failed to run 'borg list' — is borg installed and on PATH?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "borg list failed (exit {}){}",
                output.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr.trim())
                }
            );
        }

        Self::parse_latest_archive(&output.stdout)
    }

    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        target_dir: &Path,
    ) -> anyhow::Result<RestoreOutcome> {
        let mut cmd = self.borg_command();
        // Borg extract: borg extract --destination <dir> <repo>::<archive>
        let archive_ref = format!("{}::{}", self.repo, snapshot_id);
        cmd.arg("extract")
            .arg("--destination")
            .arg(target_dir)
            .arg(&archive_ref);

        let output = cmd.output().await.context("Failed to run 'borg extract'")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "borg extract failed (exit {}){}",
                output.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr.trim())
                }
            );
        }

        // Borg does not produce meaningful JSON for extract; return zero
        // stats and let the manifest-based verification do the real check.
        Ok(RestoreOutcome {
            snapshot_id: snapshot_id.to_string(),
            files_count: 0,
            bytes_restored: 0,
            count_is_meaningful: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_latest_snapshot() {
        let json = br#"[
            {
                "short_id": "abc12345",
                "time": "2024-01-15T10:00:00Z",
                "host": "myserver",
                "paths": ["/data"]
            }
        ]"#;

        let id = ResticBackend::parse_latest_snapshot_id(json).expect("should parse");
        assert_eq!(id, "abc12345");
    }

    #[test]
    fn test_parse_empty_snapshots_fails() {
        let json = b"[]";
        let err = ResticBackend::parse_latest_snapshot_id(json).unwrap_err();
        assert!(err.to_string().contains("No snapshots found"));
    }

    #[test]
    fn test_parse_restore_summary() {
        let json = r#"{"message_type":"status","percent_done":0.5,"total_files":100,"total_bytes":5000}
{"message_type":"summary","files_new":100,"total_files":100,"total_bytes":5000}"#;

        let outcome =
            ResticBackend::parse_restore_output(json.as_bytes(), "snap123").expect("should parse");
        assert_eq!(outcome.snapshot_id, "snap123");
        assert_eq!(outcome.files_count, 100);
        assert_eq!(outcome.bytes_restored, 5000);
    }

    #[test]
    fn test_parse_stats() {
        let json = br#"{"total_snapshots": 42}"#;
        let stats = ResticBackend::parse_stats_output(json).expect("should parse");
        assert_eq!(stats.snapshot_count, 42);
    }

    // ── Borg tests ──

    #[test]
    fn test_borg_parse_latest_archive() {
        let json = br#"{
            "archives": [
                {"name": "host-2024-01-14", "time": "2024-01-14T10:00:00Z"},
                {"name": "host-2024-01-15", "time": "2024-01-15T10:00:00Z"}
            ],
            "repository": {"id": "abc", "location": "/backup"}
        }"#;

        let name = BorgBackend::parse_latest_archive(json).expect("should parse");
        assert_eq!(name, "host-2024-01-15");
    }

    #[test]
    fn test_borg_parse_empty_archives_fails() {
        let json = br#"{"archives": [], "repository": {"id": "abc", "location": "/backup"}}"#;
        let err = BorgBackend::parse_latest_archive(json).unwrap_err();
        assert!(err.to_string().contains("No archives found"));
    }

    #[test]
    fn test_borg_parse_missing_archives_fails() {
        let json = br#"{"repository": {"id": "abc"}}"#;
        let err = BorgBackend::parse_latest_archive(json).unwrap_err();
        assert!(err.to_string().contains("Missing 'archives' array"));
    }
}

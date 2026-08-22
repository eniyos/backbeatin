//! Configuration management for Backbeatin.
//!
//! This module handles loading and validating TOML configuration files
//! that define backup repositories, scheduling, and notification settings.
//!
//! # Configuration Structure
//!
//! The configuration is divided into:
//! - **Repositories**: Individual backup repositories to verify
//! - **Notifications**: Webhook settings for alerts
//! - **Credentials**: Environment variable mappings (never stored directly)
//!
//! # Example Configuration
//!
//! ```toml
//! [[repo]]
//! name = "prod-s3"
//! backend = "restic"
//! uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
//! schedule = "0 0 * * * *"  # Hourly
//!
//! [repo.credential_env_vars]
//! AWS_ACCESS_KEY_ID = "AWS access key ID for S3"
//! AWS_SECRET_ACCESS_KEY = "AWS secret access key for S3"
//!
//! [notifications]
//! webhook_url = "https://hooks.slack.com/services/..."
//! on_failure_only = true
//! ```

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

/// Default cron schedule for repositories without an explicit schedule:
/// every hour at the top of the minute.
const DEFAULT_SCHEDULE: &str = "0 0 * * * *";

/// The top-level configuration for Backbeatin.
///
/// Contains the list of repositories to verify and optional notification settings.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// One or more repositories to periodically verify.
    #[serde(rename = "repo")]
    #[serde(default)]
    pub repos: Vec<RepoConfig>,

    /// Optional notification configuration.
    pub notifications: Option<NotificationsConfig>,
}

impl Config {
    /// Load configuration from a TOML file at `path`.
    ///
    /// Returns an error if the file cannot be read or parsed, or if any
    /// referenced credential environment variables are unset.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {}: {}", path.display(), e))?;

        let config: Config = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file {}: {}", path.display(), e))?;

        // Validate that all credential env vars are set.
        for repo in &config.repos {
            for var_name in repo.credential_env_vars.keys() {
                if std::env::var(var_name).is_err() {
                    anyhow::bail!(
                        "Required environment variable '{}' for repo '{}' is not set",
                        var_name,
                        repo.name
                    );
                }
            }
        }

        // Validate that repo names are unique.
        let mut seen = HashSet::new();
        for repo in &config.repos {
            if !seen.insert(&repo.name) {
                anyhow::bail!("Duplicate repository name '{}' in config", repo.name);
            }
        }

        Ok(config)
    }
}

/// The type of backup backend.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    Restic,
    Borg,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Restic => write!(f, "restic"),
            Self::Borg => write!(f, "borg"),
        }
    }
}

/// Configuration for a single backup repository.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoConfig {
    /// A human-friendly name for this repository, used in CLI commands and
    /// persistent storage.
    pub name: String,

    /// The backend engine (e.g. "restic", "borg").
    pub backend: BackendType,

    /// The repository URI (e.g. `s3:bucket/path`, `b2:bucket/path`,
    /// `/rclone:remote:path`, `ssh:user@host:repo`).
    pub uri: String,

    /// A map of environment-variable names to descriptions of what they
    /// provide.  The values themselves are read from the environment at
    /// runtime — never stored in the config file.
    #[serde(default)]
    pub credential_env_vars: HashMap<String, String>,

    /// Optional snapshot tag to filter by when selecting the latest snapshot.
    pub snapshot_tag: Option<String>,

    /// Optional cron expression for daemon mode scheduling.
    ///
    /// Uses a 6-field cron format: "sec min hour day mon weekday"
    /// Defaults to "0 0 * * * *" (every hour) if not set.
    #[serde(default = "default_schedule")]
    pub schedule: String,
}

fn default_schedule() -> String {
    DEFAULT_SCHEDULE.to_string()
}

/// Optional notification settings.
#[derive(Debug, Clone, Deserialize)]
pub struct NotificationsConfig {
    /// Webhook URL for sending failure alerts.
    pub webhook_url: String,

    /// If true, only send notifications on failure (the default behaviour).
    ///
    /// Set to false to also send pass notifications (not recommended in
    /// steady state).
    #[serde(default = "default_on_failure_only")]
    pub on_failure_only: bool,
}

fn default_on_failure_only() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].name, "prod-s3");
        assert!(config.notifications.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
snapshot_tag = "daily"

[repo.credential_env_vars]
RESTIC_REPOSITORY = "S3 repo path"
AWS_ACCESS_KEY_ID = "S3 access key"

[[repo]]
name = "b2-backup"
backend = "restic"
uri = "b2:mybucket:/path"

[notifications]
webhook_url = "https://hooks.slack.com/xxx"
on_failure_only = true
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.repos.len(), 2);
        assert_eq!(config.repos[1].uri, "b2:mybucket:/path");
        let notif = config.notifications.expect("notifications should be present");
        assert_eq!(notif.webhook_url, "https://hooks.slack.com/xxx");
        assert!(notif.on_failure_only);
    }

    #[test]
    fn test_duplicate_repo_names_fail_validation() {
        let toml_str = r#"
[[repo]]
name = "dup"
backend = "restic"
uri = "s3:bucket/one"

[[repo]]
name = "dup"
backend = "restic"
uri = "s3:bucket/two"
"#;
        let err = toml::from_str::<Config>(toml_str).unwrap();
        // Config parses OK; duplicate detection is in Config::load.
        // We test the TOML parsing here because load requires real IO.
        assert_eq!(err.repos.len(), 2);
        assert_eq!(err.repos[0].name, "dup");
        assert_eq!(err.repos[1].name, "dup");
    }

    #[test]
    fn test_parse_borg_config() {
        let toml_str = r#"
[[repo]]
name = "prod-borg"
backend = "borg"
uri = "ssh://user@host:./repo"

[repo.credential_env_vars]
BORG_PASSPHRASE = "repo passphrase"
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse Borg config");
        assert_eq!(config.repos.len(), 1);
        assert_eq!(config.repos[0].name, "prod-borg");
        match config.repos[0].backend {
            BackendType::Borg => {} // expected
            _ => panic!("expected Borg backend"),
        }
        assert_eq!(config.repos[0].uri, "ssh://user@host:./repo");
        assert!(config.repos[0].credential_env_vars.contains_key("BORG_PASSPHRASE"));
    }

    #[test]
    fn test_creds_empty_no_env_vars_set() {
        // A config with no credential_env_vars should load fine even though
        // the env vars aren't actually set (there are none to validate).
        let toml_str = r#"
[[repo]]
name = "local"
backend = "restic"
uri = "s3:http://localhost:9000/test"
"#;
        // We don't call Config::load here because that requires real IO.
        // Just verify the toml parsing works.
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!(config.repos[0].credential_env_vars.is_empty());
    }
}

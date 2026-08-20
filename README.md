# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral Docker sandbox, cryptographically verifies the result, signs each run with an Ed25519 key, and alerts on failure via webhook.

**Positioning:** A DevOps/platform engineer who already runs `restic` or `borg` via cron against S3/B2/rsync.net/BorgBase, and wants automated proof that restores actually work — without migrating their backup storage anywhere.

## Quickstart

### 1. Install

**Easiest: Package Manager Installation**

**macOS (Homebrew):**
```bash
brew tap eniyos/backbeatin
brew install backbeatin
```

**Linux (Snap):**
```bash
snap install backbeatin
```

**Linux (curl one-liner):**
```bash
curl -sSL https://raw.githubusercontent.com/eniyos/backbeatin/main/install.sh | bash
```

**Option A: Install from source (requires Rust)**
```bash
cargo install --path .
```

**Option B: Build from source**
```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo build --release
```

**Option C: Download pre-built binary**
Download the latest release for your platform from the [GitHub releases page](https://github.com/eniyos/backbeatin/releases).

**Prerequisites:**
- [Rust](https://rustup.rs/) 1.70+ (if building from source)
- [Docker](https://www.docker.com/) (for sandboxed restore execution)
- `restic` or `borg` CLI (for snapshot discovery — the CLI runs locally, restore runs inside Docker)

### 2. Configure

Create a `backbeat.toml` file (see [`examples/backbeat.toml`](examples/backbeat.toml) for a full reference):

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"

[repo.credential_env_vars]
AWS_ACCESS_KEY_ID = "AWS access key ID for S3"
AWS_SECRET_ACCESS_KEY = "AWS secret access key for S3"
```

Set your credentials in the environment (never in the config file):

```bash
export AWS_ACCESS_KEY_ID="AKIA…"
export AWS_SECRET_ACCESS_KEY="…"
```

### 3. Verify

```bash
backbeat verify prod-s3 -c backbeat.toml
```

This performs a real restore of the latest snapshot into an ephemeral Docker container, computes a SHA-256 manifest of every file, compares against the backend's report, and prints pass/fail.

- **Exit 0** — restore verified successfully
- **Exit 1** — verification failed or an error occurred

### 4. Daemon mode (scheduled verification)

```bash
backbeat daemon -c backbeat.toml
```

Runs verification on each repo's configured cron schedule (default: every hour). Sends Slack/generic webhook notifications on failure. Runs until Ctrl+C.

## CLI Commands

### `verify`

```bash
backbeat verify <REPO> [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-c, --config` | `backbeat.toml` | Path to config file |
| `--db-path` | `backbeat.db` | Path to SQLite store |

### `daemon`

```bash
backbeat daemon [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-c, --config` | `backbeat.toml` | Path to config file |
| `--db-path` | `backbeat.db` | Path to SQLite store |

## Configuration Reference

See [`examples/backbeat.toml`](examples/backbeat.toml) for a fully annotated example.

| Key | Description |
|-----|-------------|
| `[[repo]]` | A backup repository to verify. Repeat for multiple repos. |
| `repo.name` | Friendly name used in CLI commands and logs. |
| `repo.backend` | Backend engine: `"restic"` or `"borg"`. |
| `repo.uri` | Repository URI as recognised by the backend tool. |
| `repo.snapshot_tag` | Optional tag to filter snapshots by. |
| `repo.schedule` | Cron expression for daemon mode (6-field: `sec min hour day mon weekday`). Default: `0 0 * * * *` (hourly). |
| `repo.credential_env_vars` | Table of env-var names → descriptions. Values read from environment at runtime. Never put credentials in the config file. |
| `[notifications]` | Optional webhook notification settings. |
| `notifications.webhook_url` | URL to POST failure alerts to (Slack-compatible format). |
| `notifications.on_failure_only` | Default `true`. Set to `false` to also notify on pass. |

## How It Works

1. **Snapshot discovery** — `backbeat` runs `restic snapshots --json --latest 1` (or `borg list --json`) locally to find the most recent snapshot/archive.
2. **Sandboxed restore** — The snapshot is restored into a throwaway Docker container via the `restic/restic` or `borgbackup/borg` Docker image. The container is destroyed immediately after (force-remove), including the output bind mount.
3. **Manifest computation** — Every restored file is hashed (SHA-256) and recorded with its size and relative path.
4. **Verification** — The on-disk file count is compared against the backend's report (with 5% tolerance). A zero-file report from the backend causes a fail unless the backend doesn't report counts (Borg).
5. **Persistence** — The run (snapshot ID, status, file count, manifest, message) is stored in a local SQLite database.
6. **Signing** — Each run is signed with an Ed25519 keypair (auto-generated in `~/.backbeatin/`). The signature and public key are stored alongside the run.
7. **Notification** — In daemon mode, a Slack-formatted webhook is sent on failure (configurable).

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    backbeat-cli                       │
│  CLI parsing (clap), orchestration, daemon scheduler │
├──────────────────────────────────────────────────────┤
│                    backbeat-core                      │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐   │
│  │ Config  │ │ Backends │ │ Verify │ │  Store   │   │
│  │ (TOML)  │ │ Restic   │ │ SHA-256│ │ (SQLite) │   │
│  │         │ │ Borg     │ │ manifest│ │ signed   │   │
│  └─────────┘ └──────────┘ └────────┘ │  runs    │   │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ └──────────┘   │
│  │ Sandbox │ │  Sign    │ │ Notify │                 │
│  │ (Docker)│ │ (Ed25519)│ │(webhook)│                │
│  └─────────┘ └──────────┘ └────────┘                 │
└──────────────────────────────────────────────────────┘
```

The binary shells out to the official `restic`/`borg` CLI binaries via `tokio::process::Command`. It never reimplements repository format parsing.

## Build Phases

| Phase | What | Status |
|-------|------|--------|
| 1 | Core restore loop (config, backend trait, manifest, CLI) | ✅ |
| 2 | SQLite persistence (run history, schema) | ✅ |
| 3 | Docker sandbox (bollard, container lifecycle) | ✅ |
| 4 | Borg backend (BorgBackend, CLI dispatch) | ✅ |
| 5 | Ed25519 signing + key management | ✅ |
| 6 | Cron scheduling + webhook notifications | ✅ |
| 7 | Integration tests + hardening | ✅ |

## Development

### Running tests

```bash
# Unit tests (no external deps)
cargo test --lib

# All tests including integration (requires Docker)
cargo test
```

Integration tests spin up a real MinIO S3-compatible container, create test backups with the `restic/restic` Docker image, run `backbeat verify` against them, assert pass, corrupt the repo, and assert fail.

### Design constraints

- **Single static binary.** No required external services beyond Docker (for sandboxing) and the `restic`/`borg` CLI binaries themselves.
- **Never reimplement repository formats.** Always shell out to the official CLI binaries. Parse their `--json` output where available.
- **Read-only by design.** Never requires write access to the real backup repository.
- **Ephemeral, isolated restores only.** Every restore happens inside a throwaway Docker container destroyed immediately after verification.
- **Self-hosted, no phone-home.** No telemetry, no required SaaS backend.
- **Tamper-evident records.** Every run manifest is signed (Ed25519).
- **Zero false positives > catching every edge case.** A tool that cries wolf gets uninstalled.

## License

MIT

## Troubleshooting

### Docker connectivity issues

**Problem**: `Failed to connect to Docker daemon. Is it installed and running?`

**Solution**: Ensure Docker is running and accessible:
```bash
docker info
```

If you're on macOS or Linux, you may need to add your user to the `docker` group:
```bash
sudo usermod -aG docker $USER
newgrp docker
```

### Credential environment variables

**Problem**: `Required environment variable 'AWS_ACCESS_KEY_ID' for repo 'prod-s3' is not set`

**Solution**: Set the required environment variables before running backbeat:
```bash
export AWS_ACCESS_KEY_ID="your-access-key"
export AWS_SECRET_ACCESS_KEY="your-secret-key"
export RESTIC_PASSWORD="your-restic-password"  # if using password-protected repos
```

**Tip**: Consider using a tool like `direnv` or environment variable management to handle credentials automatically.

### Docker image pull failures

**Problem**: `Failed to pull Docker image` or network timeouts

**Solution**: Ensure you have network connectivity to Docker Hub. You can also pre-pull the required images:
```bash
docker pull restic/restic:latest
docker pull borgbackup/borg:latest
```

### Database lock errors

**Problem**: SQLite database is locked during concurrent operations

**Solution**: This can happen if multiple verification runs try to access the database simultaneously. The tool now uses proper mutex locking, but if you encounter issues, ensure you're not running multiple daemon instances against the same database file.

### Permission denied errors

**Problem**: `Permission denied` when accessing files or directories

**Solution**: Ensure the user running backbeat has appropriate permissions for:
- The configuration file (`backbeat.toml`)
- The database file (`backbeat.db`)
- The signing key directory (`~/.backbeatin/`)
- Docker socket access

## FAQ

### Why does backbeat need Docker?

Docker provides an isolated, ephemeral environment for performing actual restores. This ensures that:
- Restored files don't interfere with your system
- The restore environment is consistent across different platforms
- The tool can safely handle any file types or sizes without affecting your host system
- The restore environment is completely cleaned up after verification

### How much storage space does verification require?

Verification requires temporary space for:
- Docker container storage (minimal, just the image)
- Restored files during verification (varies by backup size)
- SQLite database (typically a few MB for historical records)

The restore happens in a temporary directory that's automatically cleaned up after verification completes.

### Can I verify backups without Docker?

Currently, Docker is required for the sandboxed restore functionality. However, you could modify the code to use alternative isolation methods like chroot or podman if needed.

### How often should I run verification?

The default schedule is hourly, but the optimal frequency depends on:
- Backup frequency: Verify at least as often as you create new backups
- Criticality: More critical systems may benefit from more frequent verification
- Resources: Consider storage and network costs of egress from your backup provider

Common schedules:
- **Hourly** (`0 0 * * * *`) - Default, good for most use cases
- **Daily** (`0 0 0 * * *`) - For less critical systems
- **Weekly** (`0 0 0 * * 0`) - For historical backups

### What happens if verification fails?

When verification fails:
1. The exit code is non-zero (useful for scripts/CI)
2. An error message is logged and printed
3. If configured, a webhook notification is sent
4. The failure is recorded in the SQLite database for historical tracking

You should investigate the failure message and check your backup repository integrity.

### Is my backup data safe during verification?

Yes. Backbeatin is **read-only** by design:
- It never writes to your backup repository
- It only reads data to perform verification
- All restore operations happen in isolated Docker containers
- Restored data is never persisted outside the container
- The container is force-removed immediately after verification

### Can I use backbeat with other backup tools?

Currently, backbeat supports Restic and Borg. Adding support for other backup tools involves:
1. Implementing the `BackupBackend` trait
2. Adding JSON parsing for the tool's snapshot/restore output
3. Adding appropriate Docker image support

The architecture is designed to make adding new backends straightforward.

## Security Considerations

### Credential Management

- **Never store credentials in config files**: Always use environment variables
- **Use minimal permissions**: Give backup credentials only read access to repositories
- **Rotate credentials regularly**: Follow your organization's credential rotation policies
- **Consider temporary credentials**: Use time-limited credentials where possible

### Signing Key Security

- The Ed25519 signing key is stored in `~/.backbeatin/` with restricted permissions (0o600 on Unix)
- The private key should be backed up securely like any other cryptographic key
- Losing the signing key will prevent verification of historical signatures
- Consider hardware security modules (HSMs) for high-security environments

### Network Security

- Ensure Docker daemon is properly secured
- Use encrypted connections (HTTPS, SSH) for backup repositories
- Consider network isolation for the verification process
- Regularly update Docker images to include security patches

### Code Integrity

- Verify releases using GPG signatures when available
- Build from source if you need to audit the code
- Keep dependencies updated: `cargo update`
- Monitor security advisories for Rust and dependencies

## Performance Considerations

### Storage Costs

Verification involves egress from your backup provider:
- **S3**: ~$0.09/GB egress (varies by region)
- **Backblaze B2**: ~$0.01/GB egress
- **MinIO/Self-hosted**: Depends on your infrastructure

For daily verification of a 100GB backup:
- S3 monthly cost: ~$270 (30 days × 100GB × $0.09)
- B2 monthly cost: ~$30 (30 days × 100GB × $0.01)

**Optimization**: Consider weekly verification for large backups or use caching strategies.

### Performance Tuning

- **Network latency**: Closer backup providers reduce verification time
- **Container startup**: Docker container startup adds ~1-2 seconds per verification
- **Manifest computation**: SHA-256 hashing scales linearly with file count and size
- **Database operations**: SQLite is fast but consider external databases for very high-volume scenarios

## Contributing

We welcome contributions! Please follow these guidelines:

### Development Setup

1. Clone the repository
2. Install Rust (1.70+)
3. Install Docker
4. Run tests: `cargo test --lib` (unit tests only)
5. Run full tests: `cargo test` (requires Docker)

### Code Style

- Follow Rust naming conventions
- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Add tests for new functionality
- Update documentation for API changes

### Adding New Backends

To add support for a new backup tool:

1. Implement the `BackupBackend` trait in `backbeat-core/src/repo.rs`
2. Add JSON parsing for snapshot discovery and restore output
3. Add Docker image support in `backbeat-core/src/sandbox.rs`
4. Add configuration options in `backbeat-core/src/config.rs`
5. Add tests in `crates/backbeat-cli/tests/`
6. Update documentation

### Submitting Changes

1. Fork the repository
2. Create a feature branch
3. Make your changes with tests
4. Ensure all tests pass
5. Submit a pull request with a clear description

## Support

- **Issues**: Report bugs and feature requests on [GitHub Issues](https://github.com/eniyos/backbeatin/issues)
- **Documentation**: This README and inline code documentation
- **Community**: Join discussions in GitHub Discussions (when available)

## Roadmap

Potential future enhancements:
- [ ] Additional backup backend support (Duplicity, Rclone, etc.)
- [ ] Web dashboard for verification history
- [ ] Integration with Prometheus/Grafana for monitoring
- [ ] Support for custom verification scripts
- [ ] Multi-tenant/organization support
- [ ] Plugin system for custom verification logic

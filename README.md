# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral Docker sandbox, cryptographically verifies the result, signs each run with an Ed25519 key, and alerts on failure via webhook.

**Positioning:** A DevOps/platform engineer who already runs `restic` or `borg` via cron against S3/B2/rsync.net/BorgBase, and wants automated proof that restores actually work — without migrating their backup storage anywhere.

## Quickstart

### 1. Install

```bash
cargo install --path .
```

Or build from source:

```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo build --release
```

**Prerequisites:**
- [Rust](https://rustup.rs/) 1.70+ (to build)
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

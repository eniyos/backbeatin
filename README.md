# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral sandbox, cryptographically verifies the result, and alerts on failure.

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

### 2. Configure

Create a `backbeat.toml` file (see [`examples/backbeat.toml`](examples/backbeat.toml) for a full reference):

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
snapshot_tag = "daily"

[repo.credential_env_vars]
RESTIC_REPOSITORY = "S3 repo path"
AWS_ACCESS_KEY_ID = "S3 access key"
AWS_SECRET_ACCESS_KEY = "S3 secret key"
B2_ACCOUNT_ID = "Backblaze account ID"
B2_ACCOUNT_KEY = "Backblaze account key"
```

Set your credentials in the environment (never in the config file):

```bash
export RESTIC_REPOSITORY="s3:https://…"
export AWS_ACCESS_KEY_ID="AKIA…"
export AWS_SECRET_ACCESS_KEY="…"
```

### 3. Verify

```bash
backbeat verify prod-s3 -c backbeat.toml
```

On success the tool prints a summary and exits 0. On failure it prints the error to stderr and exits 1.

## Configuration Reference

See [`examples/backbeat.toml`](examples/backbeat.toml) for a fully annotated example.

| Key | Description |
|-----|-------------|
| `[[repo]]` | A backup repository to verify. Repeat for multiple repos. |
| `repo.name` | A friendly name used in CLI commands and logs. |
| `repo.backend` | Backend engine: `"restic"` or `"borg"`. |
| `repo.uri` | Repository URI as recognised by the backend tool. |
| `repo.snapshot_tag` | Optional tag to filter snapshots by. |
| `repo.credential_env_vars` | Table of env-var names → descriptions. Values are read from the environment at runtime. |
| `[notifications]` | Optional webhook notification settings. |
| `notifications.webhook_url` | URL to POST failure alerts to. |
| `notifications.on_failure_only` | Default `true`. Set to `false` to also notify on pass. |

## Architecture

```
┌──────────────────────────────────────────────────┐
│                   backbeat-cli                    │
│  (binary entrypoint: CLI parsing, orchestration)  │
├──────────────────────────────────────────────────┤
│                   backbeat-core                   │
│  (domain logic: config, backend trait, verify)    │
│                                                   │
│  BackupBackend trait ─── ResticBackend            │
│                      └── BorgBackend (planned)    │
└──────────────────────────────────────────────────┘
```

The binary shells out to the official `restic`/`borg` CLI binaries via `tokio::process::Command`. It never reimplements repository format parsing.

## Build Order

1. ✅ **Phase 1** — Core restore loop (no Docker). Verify a Restic repo into a local temp dir.
2. 🔲 **Phase 2** — Verification engine with SQLite persistence.
3. 🔲 **Phase 3** — Docker sandbox isolation via `bollard`.
4. 🔲 **Phase 4** — Borg backend support.
5. 🔲 **Phase 5** — Ed25519 signing and proof-bundle export.
6. 🔲 **Phase 6** — Cron scheduling and webhook notifications.
7. 🔲 **Phase 7** — Integration tests and hardening.

## License

MIT

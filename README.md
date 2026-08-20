# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral Docker sandbox, cryptographically verifies the result, signs each run with an Ed25519 key, and alerts on failure via webhook.

## Quick Start

### Install

```bash
cargo install --git https://github.com/eniyos/backbeatin.git
```

### Configure

Create `backbeat.toml`:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"

[repo.credential_env_vars]
AWS_ACCESS_KEY_ID = "AWS access key ID for S3"
AWS_SECRET_ACCESS_KEY = "AWS secret access key for S3"
```

Set your credentials:

```bash
export AWS_ACCESS_KEY_ID="AKIA…"
export AWS_SECRET_ACCESS_KEY="…"
```

### Verify

```bash
backbeat verify prod-s3 -c backbeat.toml
```

### Schedule (Optional)

Add to your config:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
schedule = "0 0 * * * *"  # Every hour
```

Run the daemon:

```bash
backbeat daemon -c backbeat.toml
```

## Prerequisites

- [Rust](https://rustup.rs/) 1.70+
- [Docker](https://www.docker.com/)
- `restic` or `borg` CLI

## CLI Commands

### `verify`
```bash
backbeat verify <REPO> [OPTIONS]
```

### `daemon`
```bash
backbeat daemon [OPTIONS]
```

## How It Works

1. **Snapshot discovery** — Find latest snapshot
2. **Sandboxed restore** — Restore into Docker container
3. **Manifest computation** — SHA-256 hash every file
4. **Verification** — Compare against backend report
5. **Persistence** — Store in SQLite database
6. **Signing** — Sign with Ed25519 key
7. **Notification** — Send webhook on failure

## Configuration

See [`examples/backbeat.toml`](examples/backbeat.toml) for full reference.

## License

MIT

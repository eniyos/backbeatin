# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral Docker sandbox, cryptographically verifies the result, signs each run with an Ed25519 key, and alerts on failure via webhook.

## Quick Start

### Install

**Option 1: Cargo**
```bash
cargo install backbeatin
```

**Option 2: Binary Download**
```bash
# Download from GitHub releases
wget https://github.com/eniyos/backbeatin/releases/download/v0.1.0/backbeatin-linux-x86_64.tar.gz
tar xzf backbeatin-linux-x86_64.tar.gz
sudo install backbeatin /usr/local/bin/
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
backbeatin verify prod-s3 -c backbeat.toml
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
backbeatin daemon -c backbeat.toml
```

## Prerequisites

- [Rust](https://rustup.rs/) 1.70+
- [Docker](https://www.docker.com/)
- `restic` or `borg` CLI

## CLI Commands

### `verify`
```bash
backbeatin verify <REPO> [OPTIONS]
```

### `daemon`
```bash
backbeatin daemon [OPTIONS]
```

### `demo`
Run a self-contained demo that creates a synthetic Restic repo, verifies it,
corrupts it, verifies again (fails), and exports a signed proof bundle.

```bash
backbeatin demo -o proof-bundle.json
```

Requires Docker. Creates a temporary MinIO instance.

## How It Works

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           backbeatin verify                             │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  1. LOAD CONFIG                                                         │
│     Parse backbeat.toml → resolve repo URI + credential env vars        │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  2. SNAPSHOT DISCOVERY                                                  │
│     Restic: `restic snapshots --json` → latest snapshot ID              │
│     Borg:   `borg list --json`        → latest archive name             │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  3. SANDBOXED RESTORE                                                   │
│     ┌─────────────────────────────────────────────────────────────┐     │
│     │  Docker Container (ephemeral, --rm)                         │     │
│     │  ┌───────────────────────────────────────────────────────┐  │     │
│     │  │  restic restore <id> --target /restore                │  │     │
│     │  │  OR                                                   │  │     │
│     │  │  borg extract <archive>                               │  │     │
│     │  └───────────────────────────────────────────────────────┘  │     │
│     │  Mount: read-only bind of repo credentials only             │     │
│     └─────────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  4. MANIFEST COMPUTATION                                                │
│     Walk /restore → SHA-256 every file → build manifest tree            │
│     Record: file count + total bytes                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  5. VERIFICATION                                                        │
│     Compare manifest against backend-reported stats                     │
│     ✓ File count matches                                                │
│     ✓ Byte count matches                                                │
│     ✓ Hash tree is internally consistent                                │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                        ┌───────────┴───────────┐
                        ▼                       ▼
                   ┌─────────┐            ┌─────────┐
                   │  PASS   │            │  FAIL   │
                   └────┬────┘            └────┬────┘
                        │                       │
                        ▼                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│  6. PERSIST + SIGN + NOTIFY                                             │
│     SQLite: INSERT verification run (timestamp, hash, status)           │
│     Ed25519: sign(run_id + repo + snapshot + manifest_hash + timestamps)│
│     Webhook: POST to Slack/Discord/etc. on failure                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Security Model

- **Read-only**: Backup repositories are never written to
- **Ephemeral**: Docker containers are destroyed after each restore (`--rm`)
- **Isolated**: No network access during restore, credentials mounted read-only
- **Signed**: Every verification run is cryptographically signed with Ed25519
- **Auditable**: All runs persisted to local SQLite with full provenance chain

## Configuration

See [`examples/backbeat.toml`](examples/backbeat.toml) for full reference.

## License

MIT

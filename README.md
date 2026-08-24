# Backbeatin

> **Keep your backups exactly where they are — we prove, every day, that they'd actually save you.**

Backbeatin is a single-binary Rust tool that automatically and verifiably tests whether existing Restic and Borg backup repositories can actually be restored.

It does **not** create backups. It does **not** store your data. It reads an existing repo (read-only), performs a real restore into an ephemeral Docker sandbox, cryptographically verifies the result, signs each run with an Ed25519 key, and alerts on failure via webhook.

## Quick Start

### Install

**Option 1: Homebrew** (macOS / Linux)
```bash
brew tap eniyos/backbeatin
brew install backbeatin
```

**Option 2: Cargo**
```bash
cargo install backbeatin
```

**Option 3: Bun / npm**
```bash
bun install -g backbeatin
# or: npm install -g backbeatin
```
The wrapper downloads the platform binary from the matching GitHub release
and verifies its SHA-256 against the release's `SHA256SUMS` before install.

**Option 4: Binary Download**
```bash
# Download from GitHub releases
wget https://github.com/eniyos/backbeatin/releases/download/v0.2.0/backbeatin-linux-x86_64.tar.gz
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

**Snapshot selection**: By default the latest snapshot is verified. To verify
only the latest snapshot carrying a specific tag, set `snapshot_tag`:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
snapshot_tag = "daily"   # selects the latest snapshot tagged "daily"
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

### Sampling Large Repos (Optional)

Full restores of multi-TB repos on every run are impractical. Configure a
deterministic sample for the regular schedule and keep full restores on a
separate cadence:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
schedule = "0 4 * * * *"        # sampled run daily at 04:00
full_schedule = "0 4 * * 0 *"   # full restore weekly (Sunday 04:00)
sample = "5"                    # 5% of files (or a path glob: "data/**")
```

The sample is deterministic (hashed paths, stable across runs), and drift
detection compares only runs of the same snapshot and scope. Single-shot
runs accept the same spec via `--sample`:

```bash
backbeatin verify prod-s3 --sample 5
backbeatin verify prod-s3 --sample 'data/**'
```

### Dead Man's Switch (Optional)

A crashed daemon or dead cron job produces *no* failure alert — silent
non-execution. Point `heartbeat_url` at an external watchdog (e.g.
[Healthchecks.io](https://healthchecks.io)): it is pinged on every
successful run, and the watchdog alerts when pings stop arriving.
Optionally, `max_success_age_hours` makes the daemon itself alert when a
repo has had no successful verification within the window.

```toml
[notifications]
webhook_url = "https://hooks.slack.com/services/…"
heartbeat_url = "https://hc-ping.com/<uuid>"
max_success_age_hours = 26
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

### Exit Codes

For CI and cron integration:

| Code | Meaning |
|------|---------|
| `0`  | Verification passed (restore completed and all checks succeeded) |
| `1`  | Verification failed, or an error occurred (config error, backend failure, Docker error, …) |

Exit code `1` covers both verification failures and execution/config errors;
the distinguishing details are logged to stderr. Cron users should treat any
non-zero exit as "alert" — silent non-execution is handled separately by the
dead man's switch (`heartbeat_url`).

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
│     │  Capabilities: ALL dropped · network: none (local repo)     │     │
│     │  or bridge (remote repo — needs to reach its endpoint)      │     │
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
│     ✓ Drift check: per-file SHA-256 manifest matches the previous       │
│       successful run of the same snapshot & scope (catches same-size    │
│       corruption such as bit flips)                                     │
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
│     Ed25519: sign(canonical JSON payload of run metadata)               │
│     Webhook: POST to Slack/Discord/etc. on failure                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Security Model

- **Read-only**: Backup repositories are never written to
- **Ephemeral**: Docker containers are destroyed after each restore (`--rm`)
- **Least privilege**: Every Linux capability is dropped in the container,
  and CPU, memory and process-count limits are enforced so a corrupted or
  oversized repo cannot exhaust host resources. Local filesystem repos run
  with **no network at all** (`NetworkMode: none`). Remote backends
  (S3, B2, SFTP, …) need egress to their endpoint to fetch chunks, so they
  run on Docker's default bridge network — per-host egress allowlisting
  would require external firewall tooling and is not claimed. Restore
  targets are also checked against a configurable free-space budget before
  any restore starts.
- **Credentials**: Passed to the container as environment variables
  (never written to config files); they exist only for the container's
  lifetime.
- **Signed**: Every verification run is cryptographically signed with Ed25519
- **Auditable**: All runs persisted to local SQLite with full provenance chain

## Configuration

See [`examples/backbeat.toml`](examples/backbeat.toml) for full reference.

## Release Verification

Every GitHub release ships with two extra artifacts:

- `SHA256SUMS` — SHA-256 digest of each release tarball
- `SHA256SUMS.sig` — Ed25519 signature over `SHA256SUMS`

Verify a download before installing it:

```bash
# 1. Fetch the release public key from the repository (pinned copy below).
#    For maximum trust, obtain it out-of-band or compare its fingerprint
#    against the one published in the repo.
curl -fsSLO https://raw.githubusercontent.com/eniyos/backbeatin/main/docs/release-key.pub

# 2. Verify the Ed25519 signature over SHA256SUMS (requires openssl >= 3.x).
openssl pkeyutl -verify -rawin -pubin \
  -inkey release-key.pub \
  -in SHA256SUMS \
  -sigfile SHA256SUMS.sig

# 3. Verify the tarball checksums.
sha256sum -c SHA256SUMS
```

### Key Management

- **Private key**: The Ed25519 release signing private key lives *only* in
  the GitHub repository secret `RELEASE_SIGNING_KEY` (base64-encoded PEM).
  It is never committed to the repository, never written to disk on CI
  runners beyond the signing step (deleted immediately after signing), and
  no human routinely has access to it.
- **Public key distribution**: The public key is committed to the repository
  at [`docs/release-key.pub`](docs/release-key.pub). Anyone can pin a copy
  of this file and verify every release against it. Because it is committed,
  any change to it is visible in the repository history.
- **Rotation procedure**:
  1. Generate a new key pair: `openssl genpkey -algorithm ed25519 -out new.key.pem`
  2. Extract the public key: `openssl pkey -in new.key.pem -pubout -out docs/release-key.pub`
  3. Commit the new public key, and update the `RELEASE_SIGNING_KEY` secret
     with `base64 -i new.key.pem`.
  4. The next release is signed with the new key. Releases signed with the
     old key remain verifiable by checking out the repository at that
     release's tag (the historical `release-key.pub` is preserved in git).
  5. Securely destroy the old private key. If rotation was caused by a
     suspected compromise, re-verify all releases you still distribute and
     disclose which releases were signed by the compromised key.

## License

MIT

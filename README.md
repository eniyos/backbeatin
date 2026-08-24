# Backbeatin

> **Your backups are only as good as your last restore. Backbeatin performs that restore — daily, automatically, with cryptographic proof.**

Backbeatin verifies that existing Restic and Borg repositories are actually
restorable. It doesn't create backups and never touches your data: it reads
the repo read-only, performs a real restore into an ephemeral Docker sandbox,
hashes every file, signs the result with Ed25519, and alerts you the moment
something stops adding up.

```bash
backbeatin verify prod-s3
✓ restored 12,431 files (2.3 GiB) · manifest verified · drift check passed
✓ run signed · proof stored
```

## Install

| Channel | Command |
|---------|---------|
| Homebrew (macOS / Linux) | `brew install eniyos/backbeatin/backbeatin` |
| Bun / npm | `bun install -g backbeatin` |
| Cargo | `cargo install backbeatin` |
| Binary | Signed tarballs on [GitHub Releases](https://github.com/eniyos/backbeatin/releases) — see [Release Verification](#release-verification) |

Requires Docker and the `restic` or `borg` CLI. The npm wrapper downloads
the platform binary from the matching release and verifies it against the
signed `SHA256SUMS` manifest before install.

## Quick Start

**1. Configure** — create `backbeat.toml`:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"

[repo.credential_env_vars]
AWS_ACCESS_KEY_ID = "AWS access key ID"
AWS_SECRET_ACCESS_KEY = "AWS secret access key"
```

**2. Verify** — one real restore, end to end:

```bash
export AWS_ACCESS_KEY_ID="AKIA…"
export AWS_SECRET_ACCESS_KEY="…"
backbeatin verify prod-s3 -c backbeat.toml
```

**3. Automate** — verify on a schedule, alert on failure:

```toml
[[repo]]
name = "prod-s3"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/bucket/prod"
schedule = "0 0 * * * *"               # hourly

[notifications]
webhook_url = "https://hooks.slack.com/services/…"
```

```bash
backbeatin daemon -c backbeat.toml
```

Full reference: [`examples/backbeat.toml`](examples/backbeat.toml).

## Built for production

**Drift detection** — every run is compared against the previous successful
run of the same snapshot. Same-size corruption (bit flips, silent
overwrites) that checksum-by-count alone would miss gets flagged.

**Sampling** — multi-TB repos don't need a full restore every day:

```toml
schedule = "0 4 * * * *"        # sampled run daily at 04:00
full_schedule = "0 4 * * 0 *"   # full restore weekly (Sunday 04:00)
sample = "5"                    # 5% of files (or a glob: "data/**")
```

The sample is deterministic — hashed paths, stable across runs.

**Dead man's switch** — a crashed daemon sends no failure alert. Point
`heartbeat_url` at a watchdog like [Healthchecks.io](https://healthchecks.io);
it's pinged on every successful run, and the watchdog alerts when pings stop.
`max_success_age_hours` makes the daemon alert on its own when a repo goes
quiet.

**Snapshot selection** — verify the latest snapshot carrying a specific tag:

```toml
snapshot_tag = "daily"
```

**Signed proofs** — every run is signed (Ed25519 over a canonical JSON
payload) and persisted to local SQLite, building a tamper-evident audit
chain you can hand to compliance.

**CI-friendly exit codes**

| Code | Meaning |
|------|---------|
| `0` | Verification passed |
| `1` | Verification failed, or an error occurred (details on stderr) |

## How It Works

1. **Read-only access** — snapshot discovery via the backend CLI; the repo is never written to
2. **Sandboxed restore** — a real restore into an ephemeral Docker container (`--rm`), all capabilities dropped, CPU / memory / PID limits enforced, free-space budget checked before restore starts
3. **Manifest** — every restored file hashed with SHA-256
4. **Verification** — file count, byte count and hash tree checked against backend stats; drift-checked against the previous successful run of the same snapshot & scope
5. **Proof** — the run is persisted to SQLite, signed with Ed25519, and failure webhooks fire

## Security Model

- **Read-only** — backup repositories are never modified
- **Ephemeral** — containers are destroyed after every restore; credentials exist only as container environment variables, never in config files
- **Least privilege** — all Linux capabilities dropped; resource limits prevent a corrupted or oversized repo from exhausting the host
- **Honest network policy** — local repos run with **no network at all**; remote backends (S3, B2, SFTP, …) run on Docker's default bridge, since they must reach their endpoint to fetch chunks. Per-host egress allowlisting requires external firewall tooling and is not claimed
- **Signed & auditable** — every run Ed25519-signed and persisted with full provenance

## Release Verification

Every GitHub release ships `SHA256SUMS` and its Ed25519 signature
`SHA256SUMS.sig`. Verify before installing binaries by hand:

```bash
curl -fsSLO https://raw.githubusercontent.com/eniyos/backbeatin/main/docs/release-key.pub
openssl pkeyutl -verify -rawin -pubin \
  -inkey release-key.pub -in SHA256SUMS -sigfile SHA256SUMS.sig
sha256sum -c SHA256SUMS
```

The public key lives at [`docs/release-key.pub`](docs/release-key.pub) —
committed to the repo, so any change to it is visible in git history. The
private key exists only in the CI secret `RELEASE_SIGNING_KEY`; it is never
committed and is destroyed on the runner immediately after signing. To
rotate: generate a new pair, commit the new public key, update the secret —
old releases remain verifiable via their tag's copy of the key.

## Demo

See the full loop — verify a synthetic repo, corrupt it, catch the
corruption, export a signed proof bundle:

```bash
backbeatin demo -o proof-bundle.json
```

## License

MIT

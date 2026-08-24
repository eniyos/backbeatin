# Changelog

All notable changes to Backbeatin will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Default configuration file renamed from `backbeat.toml` to
  `backbeatin.toml` (`-c` flag and all docs/examples updated). The legacy
  name still works when the default is used and only `backbeat.toml`
  exists, with a deprecation warning.

## [0.2.0] - 2026-08-23

### Added
- Manifest drift detection: per-file SHA-256 comparison against the previous
  successful run of the same snapshot and scope — corrupted-but-same-size
  restores (bit flips, padded truncation) now fail verification
- Sampling mode: `--sample <percent|path-glob>` for partial restores with
  deterministic hash-based file selection; `sample` and `full_schedule`
  config keys for dual cadence in daemon mode
- Dead man's switch: `heartbeat_url` ping on every successful run for
  external watchdogs (e.g. Healthchecks.io), plus daemon-side
  `max_success_age_hours` staleness alerts
- Container resource limits: `[sandbox]` config (CPU, memory, process count)
  enforced via Docker, and a pre-flight free-space disk budget check
- Signed releases: `SHA256SUMS` + Ed25519 signature on GitHub release
  artifacts, with public key in `docs/release-key.pub`

### Changed
- Sandbox network policy is now honest and documented: `NetworkMode: none`
  for local repos, bridge network with all capabilities dropped for remote
  backends (which need egress to their endpoint)
- README Security Model, verification steps, exit codes, and key management
  documentation updated to match actual behavior

### Fixed
- README no longer claims "no network access during restore" for remote
  backends

## [0.1.0] - 2026-08-23

### Added
- Initial release of Backbeatin
- Restic backup backend support
- Borg backup backend support
- Docker sandboxed restore execution
- SHA-256 manifest computation
- SQLite persistence for verification history
- Ed25519 cryptographic signing of verification runs
- Webhook notifications (Slack-compatible) with SSRF protection
- Cron-based daemon scheduler with shared store
- Self-contained demo command (`backbeatin demo`)
- Multi-platform release binaries (Linux/macOS, x86_64/aarch64)
- Comprehensive documentation and troubleshooting guides

### Security
- Read-only access to backup repositories
- Ephemeral Docker containers for isolated restores
- Cryptographic signing of all verification records
- Secure credential handling via environment variables (never stored in config)
- Webhook SSRF guard: blocks localhost, private IPs, link-local, cloud metadata endpoints
- HTTP redirect following disabled to prevent open-redirect bypass
- Signing key generation warns about unencrypted on-disk storage

[0.1.0]: https://github.com/eniyos/backbeatin/releases/tag/v0.1.0

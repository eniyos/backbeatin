# Changelog

All notable changes to Backbeatin will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

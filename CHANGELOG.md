# Changelog

All notable changes to Backbeatin will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial release of Backbeatin
- Restic backup backend support
- Borg backup backend support
- Docker sandboxed restore execution
- SHA-256 manifest computation
- SQLite persistence for verification history
- Ed25519 cryptographic signing of verification runs
- Webhook notifications (Slack-compatible)
- Cron-based daemon scheduler
- Multi-platform installation support (Homebrew, Snap, Linux script)
- Comprehensive documentation and troubleshooting guides

### Fixed
- Thread safety issues in SQLite store with proper mutex locking
- Daemon mode placeholder - now performs actual verification
- Error handling in demo and verification flows
- Various clippy warnings and code quality improvements

### Security
- Read-only access to backup repositories
- Ephemeral Docker containers for isolated restores
- Cryptographic signing of all verification records
- Secure credential handling via environment variables

## [0.1.0] - 2026-08-20

### Initial Release
- Complete backup verification pipeline
- Support for Restic and Borg backends
- Docker sandboxed execution
- Cryptographic verification and signing
- Multi-platform distribution

[Unreleased]: https://github.com/eniyos/backbeatin/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/eniyos/backbeatin/releases/tag/v0.1.0

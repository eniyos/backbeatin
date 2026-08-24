# backbeatin (npm wrapper)

Prove your Restic/Borg backups are actually restorable.

This package is a thin wrapper around the native `backbeatin` binary. On
install, `postinstall` downloads the binary for your platform from the
matching GitHub release and verifies its SHA-256 against the release's
`SHA256SUMS` manifest before extracting it.

```bash
bun install -g backbeatin
# or
npm install -g backbeatin
```

Supported platforms: Linux (x86_64, aarch64), macOS (x86_64, aarch64).

Requires Docker, plus the `restic` or `borg` CLI. See the
[repository README](https://github.com/eniyos/backbeatin) for usage, the
security model, and full Ed25519 release-signature verification
instructions.

# Packaging and Distribution

This directory contains configuration files and scripts for distributing Backbeatin through various package managers and distribution channels.

## Installation Methods

### Homebrew (macOS)

**Install from our custom tap:**
```bash
brew tap eniyos/backbeatin
brew install backbeatin
```

**Or install directly from formula:**
```bash
brew install homebrew/backbeatin/backbeatin.rb
```

### Linux (curl one-liner)

```bash
curl -sSL https://raw.githubusercontent.com/eniyos/backbeatin/main/install.sh | bash
```

### Snap Store (Linux)

```bash
snap install backbeatin
```

### Cargo (Rust users)

```bash
cargo install backbeatin
```

### Manual Binary Download

Download the appropriate binary from the [GitHub Releases](https://github.com/eniyos/backbeatin/releases) page:

```bash
# Linux x86_64
wget https://github.com/eniyos/backbeatin/releases/download/v0.1.0/backbeat-linux-x86_64.tar.gz
tar xzf backbeat-linux-x86_64.tar.gz
sudo install backbeat /usr/local/bin/

# macOS x86_64
wget https://github.com/eniyos/backbeatin/releases/download/v0.1.0/backbeat-macos-x86_64.tar.gz
tar xzf backbeat-macos-x86_64.tar.gz
sudo install backbeat /usr/local/bin/
```

## Building Packages

### Homebrew Formula

The Homebrew formula is maintained in `homebrew/backbeatin.rb`. To test it locally:

```bash
brew install --build-from-source homebrew/backbeatin.rb
```

### Snap Package

Build the Snap package:

```bash
cd snap
snapcraft
```

### GitHub Releases

Releases are automatically built and published using GitHub Actions when you push a version tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow will:
1. Build binaries for multiple platforms (Linux x86_64/aarch64, macOS x86_64/aarch64)
2. Create GitHub release with all binaries
3. Generate release notes automatically

## Package Manager Support Status

| Package Manager | Status | Notes |
|----------------|--------|-------|
| Homebrew | ✅ Ready | Formula included |
| Snap | ✅ Ready | Snapcraft configuration included |
| Cargo | ✅ Ready | Published on crates.io |
| APT/RPM | 🚧 Planned | Debian/Ubuntu packages planned |
| Chocolatey | 🚧 Planned | Windows package manager |

## Prerequisites for Users

All installation methods require:
- **Docker**: For sandboxed restore execution
- **Backup CLI**: `restic` or `borg` must be installed separately

## Verification

Users can verify the authenticity of downloaded binaries using GPG signatures (when available):

```bash
gpg --verify backbeat-<version>.tar.gz.sig backbeat-<version>.tar.gz
```

## Maintenance

### Updating Homebrew Formula

When releasing a new version:

1. Update the version and SHA256 in `homebrew/backbeatin.rb`
2. Test the formula locally
3. Submit to Homebrew (or maintain in custom tap)

### Updating Snap Package

1. Update version in `snap/snapcraft.yaml`
2. Build and test locally
3. Push to Snap Store

### Release Process

1. Update version in `Cargo.toml`
2. Update CHANGELOG.md
3. Commit and tag: `git tag v0.1.0 && git push origin v0.1.0`
4. GitHub Actions will automatically build and publish
5. Update documentation with new version information
6. Announce release

## Support

For installation issues, please:
1. Check the troubleshooting section in the main README
2. Open an issue on GitHub with your OS, package manager, and error details
3. Include the output of `backbeat --version`

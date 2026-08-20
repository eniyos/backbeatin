# Packaging and Distribution

This directory contains configuration files and scripts for distributing Backbeatin through various package managers and distribution channels.

## Current Installation Status

**✅ Immediately Available:**
- **Source installation**: `git clone && cargo install --path .`
- **Build from source**: `git clone && cargo build --release`
- **Manual binary download**: From GitHub releases (when workflow completes)

**⏳ Pending Setup:**
- **Homebrew**: Formula configured, requires tap repository setup
- **Snap**: Configuration ready, requires Snap Store submission
- **Linux install script**: Ready after GitHub release completes

## Installation Methods

### Recommended: Install from Source

```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo install --path .
```

### Build from Source

```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo build --release
# Binary will be in target/release/backbeat
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

### Package Manager Installation (Future)

**Homebrew (macOS)** - Once tap repository is set up:
```bash
brew tap eniyos/backbeatin
brew install backbeatin
```

**Snap Store (Linux)** - After Snap Store submission:
```bash
snap install backbeatin
```

**Linux (curl one-liner)** - After GitHub release:
```bash
curl -sSL https://raw.githubusercontent.com/eniyos/backbeatin/main/install.sh | bash
```

## Building Packages

### Homebrew Formula

The Homebrew formula is maintained in `homebrew-tap/Formula/backbeatin.rb`. To test it locally:

```bash
brew install --build-from-source homebrew-tap/Formula/backbeatin.rb
```

**Setup Required:**
1. Create GitHub repository: `github.com/eniyos/homebrew-backbeatin`
2. Push the `homebrew-tap` directory contents
3. Users can then: `brew tap eniyos/backbeatin && brew install backbeatin`

### Snap Package

Build the Snap package:

```bash
cd snap
snapcraft
```

**Submission Required:**
1. Register on [Snapcraft](https://snapcraft.io/)
2. Upload the snap: `snapcraft upload backbeat_<version>_amd64.snap`
3. Submit for review and approval

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
4. Update Homebrew formula (when workflow is completed)

## Package Manager Support Status

| Package Manager | Status | Action Required |
|----------------|--------|-----------------|
| Source/Cargo | ✅ Ready | None - use `cargo install` |
| Manual binaries | ✅ Ready | None - download from releases |
| Homebrew | ⏳ Configured | Set up tap repository |
| Snap | ⏳ Configured | Submit to Snap Store |
| Linux script | ⏳ Configured | Wait for GitHub release |
| APT/RPM | 🚧 Planned | Future implementation |
| Chocolatey | 🚧 Planned | Future implementation |

## Prerequisites for Users

All installation methods require:
- **Docker**: For sandboxed restore execution
- **Backup CLI**: `restic` or `borg` must be installed separately

## Maintenance

### Updating Homebrew Formula

When releasing a new version:

1. Update the version and SHA256 in `homebrew-tap/Formula/backbeatin.rb`
2. Commit and push to the tap repository
3. Formula auto-updates on user's next `brew update`

### Updating Snap Package

1. Update version in `snap/snapcraft.yaml`
2. Build and test locally: `snapcraft`
3. Upload new version to Snap Store
4. Wait for review and approval

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

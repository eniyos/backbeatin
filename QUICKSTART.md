# Quick Start Guide

Get started with Backbeatin in 5 minutes.

## Installation

### Option 1: Install from Source (Recommended)

```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo install --path .
```

### Option 2: Build from Source

```bash
git clone https://github.com/eniyos/backbeatin.git
cd backbeatin
cargo build --release
sudo install target/release/backbeatin /usr/local/bin/
```

## Prerequisites

Before using Backbeatin, ensure you have:

1. **Docker** installed and running
   ```bash
   docker --version
   ```

2. **Backup CLI** installed (either restic or borg)
   ```bash
   restic version  # or
   borg version
   ```

## Your First Verification

### 1. Create Configuration

Create a file called `backbeatin.toml`:

```toml
[[repo]]
name = "my-backup"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/my-bucket/restic"

[repo.credential_env_vars]
AWS_ACCESS_KEY_ID = "Your AWS access key"
AWS_SECRET_ACCESS_KEY = "Your AWS secret key"
```

### 2. Set Credentials

```bash
export AWS_ACCESS_KEY_ID="your-key"
export AWS_SECRET_ACCESS_KEY="your-secret"
```

### 3. Run Verification

```bash
backbeatin verify my-backup -c backbeatin.toml
```

If successful, you'll see:
```
✅ VERIFICATION PASSED: Restore verified successfully: X files, Y bytes restored from snapshot <id>
```

## Schedule Automatic Verification

Add a schedule to your configuration:

```toml
[[repo]]
name = "my-backup"
backend = "restic"
uri = "s3:https://s3.us-east-1.amazonaws.com/my-bucket/restic"
schedule = "0 0 * * * *"  # Every hour

[repo.credential_env_vars]
AWS_ACCESS_KEY_ID = "Your AWS access key"
AWS_SECRET_ACCESS_KEY = "Your AWS secret key"
```

Run the daemon:

```bash
backbeatin daemon -c backbeatin.toml
```

## Troubleshooting

### Docker not running
```bash
# Start Docker
docker info
```

### Missing credentials
```bash
# Ensure environment variables are set
echo $AWS_ACCESS_KEY_ID
```

### Permission denied
```bash
# Ensure you have Docker permissions
sudo usermod -aG docker $USER
newgrp docker
```

## Next Steps

- Read the full [README.md](README.md) for detailed configuration
- See [CHANGELOG.md](CHANGELOG.md) for release notes
- Open an issue on GitHub if you encounter problems

## Support

- **Documentation**: [https://github.com/eniyos/backbeatin](https://github.com/eniyos/backbeatin)
- **Issues**: [https://github.com/eniyos/backbeatin/issues](https://github.com/eniyos/backbeatin/issues)
- **License**: MIT

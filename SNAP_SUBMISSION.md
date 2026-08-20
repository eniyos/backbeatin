# Snap Store Submission Guide

This guide will help you submit the Backbeatin Snap package to the Snap Store.

## Prerequisites

1. **Snapcraft installed**
   ```bash
   sudo snap install snapcraft --classic
   ```

2. **Snapcraft account**
   - Register at https://snapcraft.io/
   - Verify your email address
   - Accept the Snap Store terms and conditions

3. **Git account** (for snapcraft login)

## Step 1: Build the Snap Package

From the Backbeatin repository root:

```bash
cd snap
snapcraft
```

This will:
- Build the snap in a clean environment
- Create a file like `backbeat_0.1.0_amd64.snap`
- Take several minutes to complete

## Step 2: Register the Snap

If you haven't registered the snap name yet:

```bash
snapcraft register backbeat
```

## Step 3: Upload to Snap Store

```bash
snapcraft upload backbeat_0.1.0_amd64.snap
```

You'll be prompted to:
- Select the snap (backbeat)
- Choose a channel (stable)
- Add release notes

## Step 4: Submit for Review

After upload, the snap will be in "Draft" status. Submit it for review:

1. Go to https://snapcraft.io/eniyos/backbeat
2. Navigate to the snap page
3. Click "Review and publish"
4. Fill in the review form:
   - **Category**: System Tools
   - **Description**: "Automatically verify that your Restic and Borg backups can actually be restored"
   - **Summary**: "Backup verification tool for Restic and Borg"
   - **Icon**: Upload a 256x256 PNG icon
   - **Screenshots**: Add 2-3 screenshots of the tool in action
5. Submit for review

## Step 5: Wait for Approval

The Snap Store team will review your submission. This typically takes 1-3 business days. They check for:
- Security (confinement, plugs)
- Functionality
- Code quality
- Documentation

## Step 6: Users Can Install

Once approved, users can install:

```bash
snap install backbeat
```

## Snap Configuration Details

Our snap is configured with:
- **Base**: core20 (Ubuntu 20.04 LTS)
- **Confinement**: strict (highest security)
- **Plugs**: docker, network, home (required for functionality)
- **Grade**: stable (production quality)

## Troubleshooting

### Build Fails
```bash
# Ensure Docker is running
docker info

# Clean build artifacts
snapcraft clean
# Try again
snapcraft
```

### Upload Fails
```bash
# Ensure you're logged in
snapcraft login

# Check snapcraft status
snapcraft whoami
```

### Review Rejected
Common reasons:
- **Insufficient documentation**: Add more details to description
- **Security concerns**: Review confinement and plugs
- **Missing icons/screenshots**: Add required assets
- **Technical issues**: Fix build warnings or errors

## Multi-Architecture Builds

For better coverage, build for multiple architectures:

```bash
# Build for amd64
snapcraft

# Build for arm64 (requires ARM64 build environment)
snapcraft --target-arch=arm64
```

Then upload both versions to the Snap Store.

## Maintenance

**Updating the snap:**
1. Update version in `snap/snapcraft.yaml`
2. Build new version: `snapcraft`
3. Upload: `snapcraft upload backbeat_<version>_amd64.snap`
4. Update release notes in Snap Store

**GitHub Actions Integration:**
Consider adding a GitHub Actions workflow to automatically build and upload snaps to the Snap Store using snapcraft build tokens.

## Verification

After approval, test installation:

```bash
# Remove any existing test installation
snap remove backbeat

# Install from Snap Store
snap install backbeat

# Test functionality
backbeat --help
backbeat demo -o test.json
```

## Support

- **Snapcraft documentation**: https://snapcraft.io/docs/
- **Snap Store forum**: https://forum.snapcraft.io/
- **Snapcraft support**: support@snapcraft.io

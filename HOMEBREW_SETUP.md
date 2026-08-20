# Homebrew Tap Setup Instructions

This guide will help you set up the Homebrew tap repository for Backbeatin.

## Prerequisites

- GitHub account
- Git installed
- Homebrew installed (for testing)

## Step 1: Create the Tap Repository

1. Go to GitHub and create a new repository:
   - Repository name: `homebrew-backbeatin`
   - Description: "Homebrew tap for Backbeatin backup verification tool"
   - Make it public
   - Initialize with README

2. Clone the new repository:
   ```bash
   git clone https://github.com/eniyos/homebrew-backbeatin.git
   cd homebrew-backbeatin
   ```

3. Copy the tap structure from the main Backbeatin repository:
   ```bash
   # From the backbeatin directory
   cp -r homebrew-tap/* ../homebrew-backbeatin/
   ```

4. Commit and push:
   ```bash
   git add .
   git commit -m "Initial Homebrew tap for Backbeatin"
   git push origin main
   ```

## Step 2: Test the Tap

1. Install from your tap:
   ```bash
   brew tap eniyos/backbeatin https://github.com/eniyos/homebrew-backbeatin.git
   brew install backbeatin
   ```

2. Verify installation:
   ```bash
   backbeat --version
   backbeat --help
   ```

## Step 3: Update Main Repository

1. Update the main Backbeatin README to reference the tap:
   ```bash
   # In the main backbeatin repository
   # Update README.md to use the actual tap URL
   ```

2. Test the installation instructions from the README.

## Step 4: Automatic Updates

The Homebrew formula will be automatically updated when you release new versions through the GitHub Actions workflow configured in `.github/workflows/homebrew.yml`.

## Step 5: Maintenance

When releasing new versions:
1. The GitHub Actions workflow will automatically update the formula
2. Users will get updates when they run `brew update`
3. No manual intervention required for formula updates

## Troubleshooting

### Tap Not Found
```bash
# Ensure the tap is properly installed
brew tap list
# If not listed, re-add it:
brew tap eniyos/backbeatin https://github.com/eniyos/homebrew-backbeatin.git
```

### Formula Installation Fails
```bash
# Ensure Rust and Docker are installed
brew install rust docker
# Try building from source:
brew install --build-from-source backbeatin
```

## Alternative: Submit to Homebrew Core

Instead of maintaining a custom tap, you can submit the formula to Homebrew core:

1. Fork the Homebrew/homebrew-core repository
2. Add your formula to `Formula/`
3. Submit a pull request
4. After approval, users can install with: `brew install backbeatin`

This is more complex but provides broader distribution.

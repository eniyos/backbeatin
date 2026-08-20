#!/bin/bash
# Installation script for Backbeatin on Linux
# Usage: curl -sSL https://raw.githubusercontent.com/eniyos/backbeatin/main/install.sh | bash

set -e

VERSION="${VERSION:-latest}"
REPO="eniyos/backbeatin"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

echo "Installing Backbeatin ${VERSION}..."

# Detect architecture
ARCH=$(uname -m)
case $ARCH in
    x86_64)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    aarch64)
        TARGET="aarch64-unknown-linux-gnu"
        ;;
    *)
        echo "Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

# Get latest version if not specified
if [ "$VERSION" = "latest" ]; then
    VERSION=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
    echo "Latest version: ${VERSION}"
fi

# Download release
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/backbeat-linux-${ARCH}.tar.gz"
echo "Downloading from ${DOWNLOAD_URL}..."

TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"

curl -sSL -o backbeat.tar.gz "$DOWNLOAD_URL"
tar xzf backbeat.tar.gz

# Install binary
sudo install -m 755 backbeat "$INSTALL_DIR/backbeat"

# Cleanup
cd -
rm -rf "$TMP_DIR"

echo "Backbeatin ${VERSION} installed successfully to ${INSTALL_DIR}/backbeat"
echo "Run 'backbeat --help' to get started"

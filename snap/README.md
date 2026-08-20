# Snap Package for Backbeatin

This directory contains the Snapcraft configuration for building the Backbeatin Snap package.

## Building the Snap

To build the Snap package locally:

```bash
cd snap
snapcraft
```

This will create a `.snap` file that can be uploaded to the Snap Store.

## Requirements

- Snapcraft installed on your system
- Docker (for building in a clean environment)

## Installing Snapcraft

```bash
sudo snap install snapcraft --classic
```

## Submission to Snap Store

1. Build the snap: `snapcraft`
2. Register on [Snapcraft](https://snapcraft.io/)
3. Upload the snap: `snapcraft upload backbeat_<version>_amd64.snap`
4. Submit for review

## Snap Configuration

The snap is configured with:
- **Confinement**: strict (for security)
- **Base**: core20 (Ubuntu 20.04 LTS)
- **Plugs**: docker, network, home (for functionality)

## Testing the Snap

After building, you can test locally:

```bash
sudo snap install --dangerous backbeat_<version>_amd64.snap
backbeat --help
```

## Notes

- The snap includes Docker support via the docker plug
- Rust is installed during the build process
- The snap is built from source to ensure compatibility

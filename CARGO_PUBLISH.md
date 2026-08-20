# Cargo (crates.io) Publishing Guide

This guide will help you publish Backbeatin to crates.io for easy installation via `cargo install`.

## Prerequisites

1. **Rust and Cargo installed**
   ```bash
   rustc --version
   cargo --version
   ```

2. **crates.io account**
   - Register at https://crates.io/
   - Verify your email address
   - Accept the terms of service

3. **API Token**
   - Generate an API token at https://crates.io/settings/tokens
   - Use the token for `cargo login`

## Step 1: Login to crates.io

```bash
cargo login
```

When prompted, paste your API token from crates.io settings.

## Step 2: Verify Package Configuration

Check your `Cargo.toml` files to ensure they're properly configured:

**Main Cargo.toml:**
```toml
[workspace]
resolver = "2"
members = [
    "crates/backbeat-core",
    "crates/backbeat-cli",
    "crates/backbeat-bench",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
repository = "https://github.com/eniyos/backbeatin"
homepage = "https://github.com/eniyos/backbeatin"
authors = ["eniyos"]
keywords = ["backup", "restic", "borg", "verification", "devops"]
categories = ["command-line-utilities", "development-tools"]
```

**CLI Cargo.toml:**
```toml
[package]
name = "backbeat-cli"
version = "0.1.0"
edition = "2021"
license = "MIT"

[[bin]]
name = "backbeat"
path = "src/main.rs"
```

## Step 3: Dry Run Publication

Test the publishing process without actually publishing:

```bash
cargo publish --dry-run
```

This will check:
- Package metadata is valid
- All dependencies are available
- No publishing errors would occur

## Step 4: Publish

Publish the CLI package (the main binary):

```bash
cd crates/backbeat-cli
cargo publish
```

Note: Since this is a workspace, you typically publish the CLI package which depends on the other workspace crates.

## Step 5: Verify Publication

After publishing, verify it's available:

```bash
cargo search backbeat
cargo install backbeat-cli
```

## Step 6: Update Documentation

Update README and documentation to include cargo installation:

```bash
cargo install backbeat-cli
```

## Maintenance

**Publishing new versions:**
1. Update version in all `Cargo.toml` files
2. Update CHANGELOG.md
3. Dry run: `cargo publish --dry-run`
4. Publish: `cargo publish`
5. Verify installation works

**Version Guidelines:**
- Follow semantic versioning (MAJOR.MINOR.PATCH)
- Update CHANGELOG.md for each release
- Ensure workspace version consistency

## Troubleshooting

### Authentication Failed
```bash
# Logout and login again
cargo logout
cargo login
# Re-enter your API token
```

### Package Name Taken
- Choose a different package name
- Use `cargo search` to check availability
- Consider adding a prefix or suffix

### Dependency Issues
- Ensure all dependencies are published to crates.io
- Check for any path dependencies that might cause issues
- Review workspace configuration

### Publishing Blocked
- Check if you've exceeded publish rate limits
- Verify your account is in good standing
- Contact crates.io support if needed

## Workspace Considerations

Backbeatin uses a workspace structure. When publishing:

1. **Publish library crates first** (if they're meant to be used separately):
   ```bash
   cd crates/backbeat-core
   cargo publish
   ```

2. **Then publish the CLI**:
   ```bash
   cd crates/backbeat-cli
   cargo publish
   ```

3. **Or publish CLI only** (recommended for this project):
   ```bash
   cd crates/backbeat-cli
   cargo publish
   ```

## Users Can Install

Once published, users can install:

```bash
cargo install backbeat-cli
```

This will:
- Download the binary
- Compile from source
- Install to `~/.cargo/bin/backbeat`
- Add to PATH automatically

## Alternative: Publish from GitHub

Instead of crates.io, you can also publish from GitHub:

```bash
cargo install --git https://github.com/eniyos/backbeatin.git
```

This works without crates.io publishing but is less discoverable.

## Support

- **crates.io documentation**: https://doc.rust-lang.org/cargo/reference/publishing.html
- **crates.io policies**: https://crates.io/policies
- **cargo issues**: https://github.com/rust-lang/cargo/issues

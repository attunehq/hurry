# _hurry!_

Really, really fast builds.

## Usage

```bash
# Instead of `cargo build`:
$ hurry cargo build
```

# Installation

Hurry provides easy-to-use installation scripts for macOS and Linux.

> [!NOTE]
> Windows is not yet supported, but is planned.

## Quick Install

Install the latest version with:

```shell
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash
```

The installer will:
- Detect your platform and architecture automatically
- Download the appropriate binary from S3
- Verify checksums for security
- Install to `~/.local/bin` by default

### Installation Options

```shell
# Install to a specific directory
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -b /usr/local/bin

# Install a specific version
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -v 0.2.0

# Get help
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -h
```

## Supported Platforms

Pre-compiled binaries are available for:

| Platform | Architecture | Target Triple |
|----------|--------------|---------------|
| macOS | x86_64 (Intel) | `x86_64-apple-darwin` |
| macOS | ARM64 (Apple Silicon) | `aarch64-apple-darwin` |
| Linux | x86_64 (glibc) | `x86_64-unknown-linux-gnu` |
| Linux | ARM64 (glibc) | `aarch64-unknown-linux-gnu` |
| Linux | x86_64 (musl) | `x86_64-unknown-linux-musl` |
| Linux | ARM64 (musl) | `aarch64-unknown-linux-musl` |

## Manual Installation

You can also download pre-compiled binaries directly:

```shell
# View available versions
curl -sSfL https://hurry-releases.s3.amazonaws.com/releases/versions.json

# Download a specific version for your platform
curl -sSfL https://hurry-releases.s3.amazonaws.com/releases/v0.2.0/hurry-aarch64-apple-darwin.tar.gz -o hurry.tar.gz

# Extract and install
tar -xzf hurry.tar.gz
sudo mv hurry-aarch64-apple-darwin/hurry /usr/local/bin/
```

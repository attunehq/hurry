# _hurry!_

Really, really fast builds.

## Usage

```bash
# Instead of `cargo build`:
$ hurry cargo build
```

# Installation

Hurry provides easy-to-use installation scripts for all major platforms.

## Quick Install

### Unix (macOS/Linux)

Install the latest version with:

```shell
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash
```

The installer will:
- Detect your platform and architecture automatically
- Download the appropriate binary from S3
- Verify checksums for security
- Install to `~/.local/bin` by default

#### Installation Options

```shell
# Install to a specific directory
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -b /usr/local/bin

# Install a specific version
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -v 0.2.0

# Get help
curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash -s -- -h
```

### Windows

Install the latest version with PowerShell:

```powershell
irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex
```

The installer will:
- Detect your architecture automatically
- Download the appropriate binary from S3
- Verify checksums for security
- Install to `$env:LOCALAPPDATA\Programs\hurry` by default
- Automatically add to your PATH

#### Installation Options

```powershell
# Install a specific version
$env:Version="0.2.0"; irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex

# Install to a custom directory
$env:BinDir="C:\Tools"; irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex

# Show help
$env:Help="true"; irm https://hurry-releases.s3.amazonaws.com/install.ps1 | iex
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
| Windows | x86_64 | `x86_64-pc-windows-gnu` |

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

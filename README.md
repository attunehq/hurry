# _hurry!_

Really, really fast builds.

## Usage

```bash
# Instead of `cargo build`:
$ hurry cargo build
```

# Installation

Hurry provides easy-to-use installation scripts for macOS and linux.

> [!NOTE]
> Windows is not yet supported, but is planned.

## macOS / linux

Install the latest version with:

```shell
curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash
```

### Options:

```shell
# Install to a specific directory
curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -b /usr/local/bin

# Install a specific version
curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -v v0.1.0

# Get help
curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -h
```

## Manual installation

You can also download the pre-compiled binaries from the [releases page](https://github.com/attunehq/hurry/releases)
and install them manually.

#!/usr/bin/env bash
set -euo pipefail

# hurry installer script
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash
#   curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -b /usr/local/bin
#   curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash -s -- -v v0.1.0
#
# If the repository is private, set GITHUB_TOKEN:
#   GITHUB_TOKEN=<token> curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash
#
# Options:
#   -v, --version    Specify a version (default: latest)
#   -b, --bin-dir    Specify the installation directory (default: $HOME/.local/bin)
#   -t, --tmp-dir    Specify the temporary directory (default: system temp directory)
#   -h, --help       Show help message

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Fail with an error message
fail() {
  echo -e "${RED}Error: $1${NC}" >&2
  exit 1
}

# Print an informational message
info() {
  echo -e "${GREEN}$1${NC}" >&2
}

# Print a warning message
warn() {
  echo -e "${YELLOW}Warning: $1${NC}" >&2
}

# Detect the operating system and architecture
detect_platform() {
  local kernel
  local machine
  local os
  local arch

  kernel=$(uname -s)
  machine=$(uname -m)

  case "$kernel" in
    Linux)
      os="unknown-linux"
      ;;
    Darwin)
      os="apple-darwin"
      ;;
    MINGW* | MSYS* | CYGWIN*)
      fail "Windows is not supported by this installer."
      ;;
    *)
      fail "Unsupported operating system: $kernel"
      ;;
  esac

  case "$machine" in
    x86_64 | amd64)
      arch="x86_64"
      ;;
    arm64 | aarch64)
      arch="aarch64"
      ;;
    *)
      fail "Unsupported architecture: $machine"
      ;;
  esac

  # Check for musl instead of glibc on Linux
  if [[ "$os" == "unknown-linux" ]]; then
    if [[ -e /etc/alpine-release ]] || ldd /bin/sh | grep -q musl; then
      os="$os-musl"
    else
      os="$os-gnu"
    fi
  fi

  echo "${arch}-${os}"
}

# Parse command line arguments
parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -v|--version)
        VERSION="$2"
        shift 2
        ;;
      -b|--bin-dir)
        BIN_DIR="$2"
        shift 2
        ;;
      -t|--tmp-dir)
        TMP_DIR="$2"
        shift 2
        ;;
      -h|--help)
        echo "hurry installer"
        echo
        echo "Usage: curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash [args]"
        echo
        echo "If the repository is private, set GITHUB_TOKEN:"
        echo "  GITHUB_TOKEN=<token> curl -sSfL https://raw.githubusercontent.com/attunehq/hurry/main/install.sh | bash"
        echo
        echo "Options:"
        echo "  -v, --version    Specify a version (default: latest)"
        echo "  -b, --bin-dir    Specify the installation directory (default: \$HOME/.local/bin)"
        echo "  -t, --tmp-dir    Specify the temporary directory (default: system temp directory)"
        echo "  -h, --help       Show this help message"
        exit 0
        ;;
      *)
        fail "Unknown option: $1"
        ;;
    esac
  done
}

# Get the latest version number from GitHub
get_latest_version() {
  local url="https://api.github.com/repos/attunehq/hurry/releases/latest"
  local version
  local curl_args=(-sSfL)

  # Add authentication header if GITHUB_TOKEN is set
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
  fi

  if ! version=$(curl "${curl_args[@]}" "$url" | grep -o '"tag_name": "v[^"]*"' | cut -d'"' -f4); then
    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
      fail "Failed to get latest version from GitHub. Please verify your GITHUB_TOKEN has access to the repository."
    else
      fail "Failed to get latest version from GitHub. If the repository is private, set GITHUB_TOKEN environment variable."
    fi
  fi

  echo "$version"
}

# Get download URL for a release asset from GitHub API
get_asset_download_url() {
  local version="$1"
  local asset_name="$2"
  local url="https://api.github.com/repos/attunehq/hurry/releases/tags/${version}"
  local download_url
  local curl_args=(-sSfL)

  # Add authentication header if GITHUB_TOKEN is set
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
    # For private repos, we need to use the API download URL with Accept header
    curl_args+=(-H "Accept: application/octet-stream")
  fi

  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    # For private repos, get the asset ID and use the API download endpoint
    local release_data asset_id
    local api_curl_args=(-sSfL -H "Authorization: Bearer $GITHUB_TOKEN")
    if ! release_data=$(curl "${api_curl_args[@]}" "$url"); then
      fail "Failed to get release data for $version"
    fi

    # Parse JSON to find asset ID
    if ! asset_id=$(echo "$release_data" | grep -B 2 -A 2 "\"name\": \"$asset_name\"" | grep '"id":' | head -n 1 | sed 's/.*"id": *\([0-9]*\).*/\1/'); then
      fail "Failed to find asset '$asset_name' in release $version"
    fi

    if [[ -z "$asset_id" ]]; then
      fail "Asset ID not found for $asset_name"
    fi

    download_url="https://api.github.com/repos/attunehq/hurry/releases/assets/$asset_id"
  else
    # For public repos, use direct download URL
    download_url="https://github.com/attunehq/hurry/releases/download/${version}/${asset_name}"
  fi

  echo "$download_url"
}

# Download a file
download() {
  local url="$1"
  local dest="$2"
  local curl_args=(-sSfL)

  info "Downloading to $dest"

  # Add authentication header if GITHUB_TOKEN is set and URL is from GitHub API
  if [[ -n "${GITHUB_TOKEN:-}" ]] && [[ "$url" == *"api.github.com"* ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
    curl_args+=(-H "Accept: application/octet-stream")
  elif [[ -n "${GITHUB_TOKEN:-}" ]] && [[ "$url" == *"github.com"* ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
  fi

  if ! curl "${curl_args[@]}" "$url" -o "$dest"; then
    if [[ -n "${GITHUB_TOKEN:-}" ]] && [[ "$url" == *"github.com"* ]]; then
      fail "Failed to download from GitHub. Please verify your GITHUB_TOKEN has access to the repository."
    else
      fail "Failed to download from $url"
    fi
  fi
}

# Install the binary
install_binary() {
  local platform="$1"
  local version="$2"
  local bin_dir="$3"
  local tmp_dir="$4"
  local download_url
  local checksums_url
  local archive_name="hurry-${platform}.tar.gz"
  local binary_name="hurry"

  # Construct download URLs
  download_url=$(get_asset_download_url "$version" "$archive_name")
  checksums_url=$(get_asset_download_url "$version" "checksums.txt")

  # Create temporary directory
  local workdir="$tmp_dir/hurry-install-$$"
  mkdir -p "$workdir"
  cd "$workdir"

  # Download archive and checksums
  download "$download_url" "$archive_name"
  download "$checksums_url" "checksums.txt"

  # Verify checksum
  info "Verifying checksum"
  local expected_checksum
  expected_checksum=$(grep "$archive_name" checksums.txt | awk '{print $1}')
  if [[ -z "$expected_checksum" ]]; then
    fail "Couldn't find checksum for $archive_name"
  fi

  local actual_checksum
  if command -v sha256sum > /dev/null; then
    actual_checksum=$(sha256sum "$archive_name" | awk '{print $1}')
  elif command -v shasum > /dev/null; then
    actual_checksum=$(shasum -a 256 "$archive_name" | awk '{print $1}')
  else
    fail "Neither sha256sum nor shasum found, cannot verify download"
  fi

  if [[ "$expected_checksum" != "$actual_checksum" ]]; then
    fail "Checksum verification failed! Expected: $expected_checksum, got: $actual_checksum"
  fi

  # Extract archive and binary
  tar -xzf "$archive_name"
  mkdir -p "$bin_dir"

  local extracted_binary
  extracted_binary=$(find . -name "$binary_name" -type f | head -n 1)
  if [[ -z "$extracted_binary" ]]; then
    fail "Could not find $binary_name in the extracted archive"
  fi
  cp "$extracted_binary" "$bin_dir/hurry"
  chmod +x "$bin_dir/hurry"

  # Clean up
  cd - > /dev/null
  rm -rf "$workdir"

  OUTPUT=$("$bin_dir/hurry" --version)
  info "Installed '$OUTPUT' to '$bin_dir/hurry'"

  # Check if bin_dir is in PATH
  if [[ ":$PATH:" != *":$bin_dir:"* ]]; then
    warn "'$bin_dir' is not in your PATH. You may need to add it to your shell's configuration."
  fi
}

# Main function
main() {
  # Set defaults
  local VERSION=""
  local BIN_DIR="$HOME/.local/bin"
  local TMP_DIR="${TMPDIR:-/tmp}"

  # Parse command line arguments
  parse_args "$@"

  # Detect platform
  local PLATFORM
  PLATFORM=$(detect_platform)
  info "Detected platform: $PLATFORM"

  # If version not specified, get latest
  if [[ -z "$VERSION" ]]; then
    VERSION=$(get_latest_version)
    info "Using latest version: $VERSION"
  fi

  # Install binary
  install_binary "$PLATFORM" "$VERSION" "$BIN_DIR" "$TMP_DIR"

  info "Installation complete! Run 'hurry --help' to get started."
}

# Run main function
main "$@"

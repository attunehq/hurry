#!/usr/bin/env bash
set -euo pipefail

# hurry release script
# Builds and publishes a new release to S3

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BUCKET="hurry-releases"
AWS_PROFILE="PowerUserAccess/jess@attunehq.com"
BUILD_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
)

fail() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

info() {
    echo -e "${GREEN}$1${NC}" >&2
}

warn() {
    echo -e "${YELLOW}Warning: $1${NC}" >&2
}

step() {
    echo -e "${BLUE}==>${NC} $1" >&2
}

check_requirements() {
    local missing=()

    # Check for cargo
    if ! command -v cargo > /dev/null; then
        missing+=("cargo")
    fi

    # Check for cross (only if we're not skipping build)
    if [[ "$SKIP_BUILD" == "false" ]] && ! command -v cross > /dev/null; then
        missing+=("cross")
    fi

    # Check for cargo-set-version
    if ! command -v cargo-set-version > /dev/null; then
        missing+=("cargo-set-version")
    fi

    # Check for jq
    if ! command -v jq > /dev/null; then
        missing+=("jq")
    fi

    # Check for git
    if ! command -v git > /dev/null; then
        missing+=("git")
    fi

    # Check for aws cli (only if we're not skipping upload)
    if [[ "$SKIP_UPLOAD" == "false" ]] && [[ "$DRY_RUN" == "false" ]] && ! command -v aws > /dev/null; then
        missing+=("aws")
    fi

    # Check for tar
    if ! command -v tar > /dev/null; then
        missing+=("tar")
    fi

    # Check for sha256sum
    if ! command -v sha256sum > /dev/null; then
        missing+=("sha256sum")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing required commands: ${missing[*]}

Please install the missing commands:

  cargo:              https://rustup.rs/
  cross:              cargo install cross
  cargo-set-version:  cargo install cargo-set-version
  jq:                 https://jqlang.github.io/jq/download/ (or: brew install jq, apt install jq)
  aws:                https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html
  git:                https://git-scm.com/downloads
  tar/sha256sum:      (should be pre-installed on Unix systems)"
    fi
}

usage() {
    cat <<EOF
Usage: $0 <version> [options]

Arguments:
  version          Version to release (e.g., 1.0.0 or 1.0.0-beta.1)

Options:
  --skip-build     Skip the build step (use existing artifacts)
  --skip-upload    Skip the S3 upload step
  --dry-run        Don't upload to S3 or create git tags
  -h, --help       Show this help message

Examples:
  $0 1.0.0                    # Release stable version 1.0.0
  $0 1.0.0-beta.1             # Release prerelease version 1.0.0-beta.1
  $0 1.0.0 --dry-run          # Test the release process without uploading
  $0 1.0.0 --skip-build       # Upload existing artifacts without rebuilding

Environment:
  AWS_PROFILE      AWS profile to use (default: $AWS_PROFILE)
  BUCKET           S3 bucket name (default: $BUCKET)
EOF
    exit 0
}

# Parse arguments
VERSION=""
SKIP_BUILD=false
SKIP_UPLOAD=false
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)
            usage
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --skip-upload)
            SKIP_UPLOAD=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -*)
            fail "Unknown option: $1"
            ;;
        *)
            if [[ -z "$VERSION" ]]; then
                VERSION="$1"
            else
                fail "Multiple versions specified: $VERSION and $1"
            fi
            shift
            ;;
    esac
done

# Validate version
if [[ -z "$VERSION" ]]; then
    fail "Version is required. Usage: $0 <version>"
fi

# Check if version matches semantic versioning
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-z]+\.[0-9]+)?$ ]]; then
    fail "Invalid version format: $VERSION. Expected format: X.Y.Z or X.Y.Z-prerelease.N"
fi

# Check for required commands
check_requirements

# Determine if this is a prerelease
PRERELEASE=false
if [[ "$VERSION" =~ -[a-z]+\.[0-9]+ ]]; then
    PRERELEASE=true
    info "Detected prerelease version"
fi

TAG="v$VERSION"

# Get repository root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Check for uncommitted changes
if [[ "$DRY_RUN" == "false" ]] && ! git diff-index --quiet HEAD --; then
    fail "You have uncommitted changes. Please commit or stash them before releasing."
fi

# Check if tag already exists
if git rev-parse "$TAG" >/dev/null 2>&1; then
    if [[ "$DRY_RUN" == "false" ]]; then
        fail "Tag $TAG already exists. Use a different version or delete the existing tag."
    else
        warn "Tag $TAG already exists (continuing because this is a dry run)"
    fi
fi

step "Releasing hurry version $VERSION (tag: $TAG)"

# Create temporary directory for artifacts
ARTIFACT_DIR="$REPO_ROOT/target/release-artifacts"
rm -rf "$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR"

# Update version in Cargo.toml files
step "Updating version in Cargo.toml files"
cargo set-version "$VERSION" || fail "Failed to set version"

# Build for all targets
if [[ "$SKIP_BUILD" == "false" ]]; then
    step "Building for all targets"
    for target in "${BUILD_TARGETS[@]}"; do
        info "Building for $target"

        # Determine whether to use cross or cargo
        HOST_ARCH="$(uname -m)"
        HOST_OS="$(uname -s)"
        USE_CROSS=false

        # Use cross for non-native targets
        if [[ "$HOST_OS" == "Darwin" ]]; then
            # On macOS, use cross for all Linux targets
            if [[ "$target" == *"linux"* ]]; then
                USE_CROSS=true
            fi
            # Use cross for x86_64 macOS if we're on ARM
            if [[ "$HOST_ARCH" == "arm64" ]] && [[ "$target" == "x86_64-apple-darwin" ]]; then
                USE_CROSS=false  # Actually, cargo can cross-compile between macOS architectures
            fi
        else
            # On Linux, use cross for all non-native targets
            USE_CROSS=true
        fi

        if [[ "$USE_CROSS" == "true" ]]; then
            cross build --release --target "$target" --package hurry || fail "Build failed for $target"
        else
            cargo build --release --target "$target" --package hurry || fail "Build failed for $target"
        fi

        # Package the binary
        ARCHIVE_NAME="hurry-${target}"
        ARCHIVE_DIR="$ARTIFACT_DIR/$ARCHIVE_NAME"
        mkdir -p "$ARCHIVE_DIR"

        cp "target/$target/release/hurry" "$ARCHIVE_DIR/" || fail "Failed to copy binary for $target"
        cp README.md "$ARCHIVE_DIR/" || fail "Failed to copy README"

        # Create tarball
        (cd "$ARTIFACT_DIR" && tar -czf "${ARCHIVE_NAME}.tar.gz" "$ARCHIVE_NAME") || fail "Failed to create tarball for $target"
        rm -rf "$ARCHIVE_DIR"

        info "✓ Built and packaged $target"
    done
else
    step "Skipping build (--skip-build specified)"

    # Verify artifacts exist
    for target in "${BUILD_TARGETS[@]}"; do
        ARCHIVE_NAME="hurry-${target}.tar.gz"
        if [[ ! -f "$ARTIFACT_DIR/$ARCHIVE_NAME" ]]; then
            fail "Missing artifact: $ARCHIVE_NAME. Cannot skip build."
        fi
    done
fi

# Generate checksums
step "Generating checksums"
(cd "$ARTIFACT_DIR" && sha256sum *.tar.gz > checksums.txt) || fail "Failed to generate checksums"
info "✓ Generated checksums"

# Display checksums
cat "$ARTIFACT_DIR/checksums.txt"

# Restore Cargo.toml files to avoid committing version changes
step "Restoring Cargo.toml files"
git checkout -- "**/Cargo.toml" 2>/dev/null || true

# Create git tag
if [[ "$DRY_RUN" == "false" ]]; then
    step "Creating git tag $TAG"
    git tag -a "$TAG" -m "Release $VERSION" || fail "Failed to create git tag"
    info "✓ Created tag $TAG"
else
    step "Skipping git tag creation (dry run)"
fi

# Upload to S3
if [[ "$SKIP_UPLOAD" == "false" ]]; then
    if [[ "$DRY_RUN" == "false" ]]; then
        step "Uploading to S3"

        # Upload versioned release
        info "Uploading versioned release to s3://$BUCKET/releases/$TAG/"
        aws s3 sync "$ARTIFACT_DIR/" "s3://$BUCKET/releases/$TAG/" \
            --exclude "*" --include "*.tar.gz" --include "checksums.txt" \
            --cache-control "public, max-age=31536000, immutable" \
            --profile "$AWS_PROFILE" || fail "Failed to upload to S3"

        # Update "latest" for stable releases
        if [[ "$PRERELEASE" == "false" ]]; then
            info "Updating latest release pointer"
            aws s3 sync "$ARTIFACT_DIR/" "s3://$BUCKET/releases/latest/" \
                --exclude "*" --include "*.tar.gz" --include "checksums.txt" \
                --cache-control "no-cache, must-revalidate" \
                --profile "$AWS_PROFILE" || fail "Failed to update latest release"
        else
            info "Skipping latest update (prerelease version)"
        fi

        # Update versions.json
        step "Updating versions.json"

        # Download existing versions.json (if it exists)
        VERSIONS_FILE="$ARTIFACT_DIR/versions.json"
        aws s3 cp "s3://$BUCKET/releases/versions.json" "$VERSIONS_FILE" --profile "$AWS_PROFILE" 2>/dev/null || echo '{"latest": "", "versions": []}' > "$VERSIONS_FILE"

        # Add new version to the list
        PUBLISHED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
        PLATFORMS_JSON="$(printf '%s\n' "${BUILD_TARGETS[@]}" | jq -R . | jq -s .)"

        # Update versions.json using jq
        jq --arg version "$VERSION" \
           --arg prerelease "$PRERELEASE" \
           --arg published_at "$PUBLISHED_AT" \
           --argjson platforms "$PLATFORMS_JSON" \
           '
           .versions |= ([{
               version: $version,
               prerelease: ($prerelease == "true"),
               published_at: $published_at,
               platforms: $platforms
           }] + .) |
           if ($prerelease == "false") then
               .latest = $version
           else
               .
           end
           ' "$VERSIONS_FILE" > "$VERSIONS_FILE.tmp"
        mv "$VERSIONS_FILE.tmp" "$VERSIONS_FILE"

        # Upload versions.json
        aws s3 cp "$VERSIONS_FILE" "s3://$BUCKET/releases/versions.json" \
            --cache-control "no-cache, must-revalidate" \
            --profile "$AWS_PROFILE" || fail "Failed to upload versions.json"

        # Upload install.sh to bucket root
        step "Uploading install.sh"
        aws s3 cp "$REPO_ROOT/scripts/install.sh" "s3://$BUCKET/install.sh" \
            --cache-control "no-cache, must-revalidate" \
            --profile "$AWS_PROFILE" || fail "Failed to upload install.sh"

        info "✓ Uploaded to S3"

        # Display download URLs
        echo ""
        info "Release published successfully!"
        echo ""
        echo "Install command:"
        echo "  curl -sSfL https://$BUCKET.s3.amazonaws.com/install.sh | bash"
        echo ""
        echo "Download URLs:"
        for target in "${BUILD_TARGETS[@]}"; do
            echo "  https://$BUCKET.s3.amazonaws.com/releases/$TAG/hurry-${target}.tar.gz"
        done
        echo ""
        echo "Checksums:"
        echo "  https://$BUCKET.s3.amazonaws.com/releases/$TAG/checksums.txt"
        echo ""
        echo "Versions manifest:"
        echo "  https://$BUCKET.s3.amazonaws.com/releases/versions.json"

    else
        step "Skipping S3 upload (dry run)"
        echo ""
        info "Dry run complete. Artifacts built in:"
        echo "  $ARTIFACT_DIR"
        echo ""
        echo "Would upload to:"
        echo "  s3://$BUCKET/releases/$TAG/"
        if [[ "$PRERELEASE" == "false" ]]; then
            echo "  s3://$BUCKET/releases/latest/ (latest pointer)"
        fi
    fi
else
    step "Skipping S3 upload (--skip-upload specified)"
fi

# Push git tag
if [[ "$DRY_RUN" == "false" ]]; then
    echo ""
    warn "Don't forget to push the git tag:"
    echo "  git push origin $TAG"
else
    echo ""
    info "Dry run complete. No changes made to git or S3."
fi

echo ""
info "Release process complete!"

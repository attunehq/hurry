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
AWS_PROFILE="${AWS_PROFILE:-}"

# Windows target choice: GNU vs MSVC
# We use x86_64-pc-windows-gnu instead of x86_64-pc-windows-msvc because:
# 1. GNU binaries work on Windows without requiring MSYS2/MinGW for end users
# 2. GNU cross-compiles cleanly from macOS/Linux using cross
# 3. MSVC cross-compilation via Wine fails with "command line too long" errors for large projects
# 4. Hurry is a standalone CLI tool that doesn't need MSVC-specific features or Visual Studio interop
# 5. MSVC would require building on actual Windows machines or Windows CI runners
#
# Windows ARM64 (aarch64-pc-windows-*):
# Not included because cross doesn't provide Docker images for Windows ARM64 targets, and native
# cross-compilation requires toolchains not available on macOS/Linux. The Windows ARM64 market is
# still very small, and users can either build from source or use x64 emulation (which works well
# on Windows ARM64). If this becomes important, we will need to revisit.
BUILD_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
    "x86_64-pc-windows-gnu"
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

    # Check for cargo-cross (only if we're not skipping build)
    if [[ "$SKIP_BUILD" == "false" ]] && ! command -v cargo-cross > /dev/null; then
        missing+=("cargo-cross")
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

# (gh CLI check removed; not used in script)

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
  cargo-cross:        cargo install cargo-cross
  jq:                 https://jqlang.github.io/jq/download/ (or: brew install jq, apt install jq)
  aws:                https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html
  gh:                 https://cli.github.com/ (or: brew install gh, apt install gh)
  git:                https://git-scm.com/downloads
  tar/sha256sum:      (should be pre-installed on Unix systems)"
    fi
}

check_aws_auth() {
    # Skip AWS auth check if we're skipping upload or doing a dry run
    if [[ "$SKIP_UPLOAD" == "true" ]] || [[ "$DRY_RUN" == "true" ]]; then
        return 0
    fi

    step "Checking AWS authentication"

    # Check that AWS_PROFILE is set
    if [[ -z "$AWS_PROFILE" ]]; then
        fail "AWS_PROFILE environment variable is not set.

Please set AWS_PROFILE to your AWS SSO profile:
  export AWS_PROFILE=your-profile-name"
    fi

    # Try to get AWS identity using the configured profile
    if ! aws sts get-caller-identity --profile "$AWS_PROFILE" > /dev/null 2>&1; then
        fail "AWS authentication failed for profile '$AWS_PROFILE'.

Please authenticate with AWS:
  aws sso login --profile $AWS_PROFILE"
    fi

    info "✓ AWS authentication verified"
}

usage() {
    cat <<EOF
Usage: $0 <version> [options]
       $0 --generate-changelog

Arguments:
  version          Version to release (e.g., 1.0.0 or 1.0.0-beta.1)

Options:
  --skip-build         Skip the build step (use existing artifacts)
  --skip-upload        Skip the S3 upload step
  --dry-run            Don't upload to S3 or create git tags
  --generate-changelog Generate and display changelog only (no version required)
  -h, --help           Show this help message

Examples:
  $0 1.0.0                    # Release stable version 1.0.0
  $0 1.0.0-beta.1             # Release prerelease version 1.0.0-beta.1
  $0 1.0.0 --dry-run          # Test the release process without uploading
  $0 1.0.0 --skip-build       # Upload existing artifacts without rebuilding
  $0 --generate-changelog     # Preview changelog without releasing

Environment:
  AWS_PROFILE      AWS profile to use (default: $AWS_PROFILE)
  BUCKET           S3 bucket name (default: $BUCKET)
EOF
    exit 0
}

is_user_facing_commit() {
    local commit_msg="$1"
    local commit_sha="$2"

    # Check for explicit markers first
    if [[ "$commit_msg" =~ \[skip.changelog\] ]] || [[ "$commit_msg" =~ \[internal\] ]]; then
        return 1
    fi

    if [[ "$commit_msg" =~ \[user.facing\] ]]; then
        return 0
    fi

    # Filter out non-user-facing commit types (conventional commits style)
    if [[ "$commit_msg" =~ ^(refactor|chore|ci|docs|test|style|build): ]]; then
        return 1
    fi

    # Filter out commits that mention internal tooling
    if [[ "$commit_msg" =~ (AGENTS\.md|CLAUDE\.md|\.agents/|agent guidance|Update agent) ]]; then
        return 1
    fi

    # Check changed files for internal-only changes
    local changed_files
    changed_files=$(git show --name-only --format="" "$commit_sha" 2>/dev/null)

    # Count how many files are internal vs user-facing
    local internal_count=0
    local total_count=0

    while IFS= read -r file; do
        [[ -z "$file" ]] && continue
        ((total_count++))

        # Internal files
        if [[ "$file" =~ ^\.agents/ ]] || \
           [[ "$file" =~ ^\.github/ ]] || \
           [[ "$file" =~ AGENTS\.md$ ]] || \
           [[ "$file" =~ CLAUDE\.md$ ]] || \
           [[ "$file" =~ ^scripts/release\.sh$ ]] || \
           [[ "$file" =~ ^\.scratch/ ]]; then
            ((internal_count++))
        fi
    done <<< "$changed_files"

    # If all changed files are internal, skip this commit
    if [[ $total_count -gt 0 ]] && [[ $internal_count -eq $total_count ]]; then
        return 1
    fi

    # Default: include the commit (user-facing)
    return 0
}

generate_changelog() {
    local output_file="$1"

    info "Generating changelog from commit history"

    # Get list of all tags sorted by version
    local tags
    tags=$(git tag -l 'v*' | sort -V)

    # Start the changelog
    cat > "$output_file" <<EOF
# Hurry Changelog

All notable changes to this project are documented here.

EOF

    # Process all releases and their commits
    # Convert tags to array for easier indexing
    local tags_array=()
    while IFS= read -r tag; do
        tags_array+=("$tag")
    done <<< "$tags"

    # Filter to only stable releases (non-prerelease versions)
    local stable_tags=()
    for tag in "${tags_array[@]}"; do
        local version="${tag#v}"
        # Skip prerelease versions (contain - followed by alpha, beta, rc, etc)
        if [[ ! "$version" =~ -[a-z][a-z0-9.]* ]]; then
            stable_tags+=("$tag")
        fi
    done

    # Process stable tags in reverse order (newest first)
    for ((i=${#stable_tags[@]}-1; i>=0; i--)); do
        local tag="${stable_tags[$i]}"
        local version="${tag#v}"

        # Get the tag date
        local tag_date
        tag_date=$(git log -1 --format=%ai "$tag" 2>/dev/null | cut -d' ' -f1)

        # Generate the version header
        echo "## [$version] - $tag_date" >> "$output_file"
        echo "" >> "$output_file"

        # Get commits for this version
        # Range should be from previous stable release to this stable release
        # This captures all commits including those in prerelease versions
        local commit_range
        if [[ $i -eq 0 ]]; then
            # First stable tag: get all commits up to and including this tag
            commit_range="$tag"
        else
            # Get commits between previous stable tag and this stable tag
            local prev_stable_tag="${stable_tags[$((i-1))]}"
            commit_range="$prev_stable_tag..$tag"
        fi

        # Get commits and filter for user-facing changes
        local has_commits=false
        while IFS= read -r line; do
            [[ -z "$line" ]] && continue

            local commit_sha="${line%% *}"
            local commit_msg="${line#* }"

            if is_user_facing_commit "$commit_msg" "$commit_sha"; then
                echo "- $commit_msg" >> "$output_file"
                has_commits=true
            fi
        done < <(git log "$commit_range" --pretty=format:"%H %s" --reverse 2>/dev/null)

        if [[ "$has_commits" == "false" ]]; then
            echo "- Internal changes and improvements" >> "$output_file"
        fi

        echo "" >> "$output_file"
    done

    info "✓ Generated changelog with ${#stable_tags[@]} stable releases"
}

# Parse arguments
VERSION=""
SKIP_BUILD=false
SKIP_UPLOAD=false
DRY_RUN=false
GENERATE_CHANGELOG_ONLY=false

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
        --generate-changelog)
            GENERATE_CHANGELOG_ONLY=true
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

# Handle changelog-only mode
if [[ "$GENERATE_CHANGELOG_ONLY" == "true" ]]; then
    TEMP_CHANGELOG=$(mktemp)
    if ! generate_changelog "$TEMP_CHANGELOG" 2>&1; then
        fail "Failed to generate changelog"
    fi
    cat "$TEMP_CHANGELOG"
    rm "$TEMP_CHANGELOG"
    exit 0
fi

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

# Check AWS authentication early
check_aws_auth

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

# Check that we're on main branch (skip in dry run mode)
CURRENT_BRANCH="$(git branch --show-current)"
if [[ "$DRY_RUN" != "true" ]] && [[ "$CURRENT_BRANCH" != "main" ]]; then
    fail "Releases must be created from the 'main' branch. Currently on: $CURRENT_BRANCH"
fi

# Check that main is up-to-date with remote (skip in dry run mode)
if [[ "$DRY_RUN" != "true" ]]; then
    step "Checking that main is up-to-date with origin"
    git fetch origin main || fail "Failed to fetch from origin"

    LOCAL_REV="$(git rev-parse main)"
    REMOTE_REV="$(git rev-parse origin/main)"

    if [[ "$LOCAL_REV" != "$REMOTE_REV" ]]; then
        # Check if local is ahead, behind, or diverged
        MERGE_BASE="$(git merge-base main origin/main)"
        if [[ "$LOCAL_REV" == "$MERGE_BASE" ]]; then
            fail "Your local main branch is behind origin/main. Please pull the latest changes: git pull origin main"
        elif [[ "$REMOTE_REV" == "$MERGE_BASE" ]]; then
            fail "Your local main branch is ahead of origin/main. Please push your changes: git push origin main"
        else
            fail "Your local main branch has diverged from origin/main. Please sync your branches."
        fi
    fi

    info "✓ main is up-to-date with origin/main"
fi

# Check for uncommitted changes (skip in dry run mode)
if [[ "$DRY_RUN" != "true" ]] && ! git diff-index --quiet HEAD --; then
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

# Create git tag before building so git_version! picks it up
if [[ "$DRY_RUN" == "false" ]]; then
    step "Creating git tag $TAG (before build)"
    git tag -a "$TAG" -m "Release $VERSION" || fail "Failed to create git tag"
    info "✓ Created tag $TAG"
else
    step "Skipping git tag creation (dry run)"
fi

# Build for all targets
if [[ "$SKIP_BUILD" == "false" ]]; then
    step "Building for all targets"
    for target in "${BUILD_TARGETS[@]}"; do
        info "Building for $target"

        # Use cargo-cross for all cross-compilation (it's faster than Docker-based cross)
        # cargo-cross automatically manages cross-compilers and works for all our targets
        cargo cross build --target "$target" --package hurry --release || fail "Build failed for $target"

        # Package the binary
        ARCHIVE_NAME="hurry-${target}"
        ARCHIVE_DIR="$ARTIFACT_DIR/$ARCHIVE_NAME"
        mkdir -p "$ARCHIVE_DIR"

        # Windows binaries have .exe extension
        if [[ "$target" == *"windows"* ]]; then
            cp "target/$target/release/hurry.exe" "$ARCHIVE_DIR/" || fail "Failed to copy binary for $target"
        else
            cp "target/$target/release/hurry" "$ARCHIVE_DIR/" || fail "Failed to copy binary for $target"
        fi
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

        # Generate and upload changelog (only for stable releases)
        if [[ "$PRERELEASE" == "false" ]]; then
            step "Generating and uploading changelog"
            CHANGELOG_FILE="$ARTIFACT_DIR/CHANGELOG.md"
            generate_changelog "$CHANGELOG_FILE"
            aws s3 cp "$CHANGELOG_FILE" "s3://$BUCKET/releases/CHANGELOG.md" \
                --cache-control "no-cache, must-revalidate" \
                --profile "$AWS_PROFILE" || fail "Failed to upload CHANGELOG.md"
        else
            info "Skipping changelog update (prerelease version)"
        fi

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
        echo ""
        echo "Changelog:"
        echo "  https://$BUCKET.s3.amazonaws.com/releases/CHANGELOG.md"

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

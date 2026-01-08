#!/usr/bin/env bash
set -euo pipefail

# Issue #319 Reproduction Script
# ==============================
# Reproduces the "dependency fingerprint hash not found" error that occurs
# when the cache returns a unit but not all of its dependencies.
#
# This script:
# 1. Builds hurry locally for Linux (using cargo cross) OR uses the release version
# 2. Clones the test project (github-rs at commit 3eeaef5b) into a Docker container
# 3. Runs hurry cargo build with the repro/319 org's cached artifacts
# 4. The cache returns incomplete dependency chains, triggering the bug
#
# Prerequisites:
#   - cargo-cross (cargo install cargo-cross) - only needed for local builds
#   - Docker with buildx support
#   - API token for accessing the `repro/319` org in Hurry production
#
# Usage:
#   export HURRY_API_TOKEN="<token-for-repro-319-org>"
#   ./packages/e2e/repros/319/repro.sh           # Use locally-built hurry (test fix)
#   ./packages/e2e/repros/319/repro.sh --release # Use release hurry (reproduce bug)

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

fail() { echo -e "${RED}Error: $1${NC}" >&2; exit 1; }
info() { echo -e "${GREEN}$1${NC}" >&2; }
warn() { echo -e "${YELLOW}$1${NC}" >&2; }
step() { echo -e "${BLUE}==>${NC} $1" >&2; }

# Parse arguments
USE_RELEASE=false
for arg in "$@"; do
    case $arg in
        --release)
            USE_RELEASE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--release]"
            echo ""
            echo "Options:"
            echo "  --release    Use the latest release version of hurry (to reproduce the bug)"
            echo "               Without this flag, builds and uses local hurry (to test the fix)"
            exit 0
            ;;
    esac
done

# Get repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
cd "$REPO_ROOT"

# Output directory for logs
OUTPUT_DIR="$SCRIPT_DIR/local"
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

# Check prerequisites
step "Checking prerequisites"

if [[ "$USE_RELEASE" == "false" ]]; then
    if ! command -v cargo-cross &> /dev/null; then
        fail "cargo-cross not found. Install with: cargo install cargo-cross"
    fi
fi

if ! command -v docker &> /dev/null; then
    fail "docker not found"
fi

if [[ -z "${HURRY_API_TOKEN:-}" ]]; then
    fail "HURRY_API_TOKEN environment variable not set.

This reproduction requires access to the 'repro/319' org (ID: 46) in Hurry production.
Please obtain the API token for this org and set:

  export HURRY_API_TOKEN='<token-for-repro-319-org>'

Contact the Hurry team if you need access to this org."
fi

info "Using HURRY_API_URL=https://app.hurry.build/"

if [[ "$USE_RELEASE" == "true" ]]; then
    info "Mode: Using RELEASE hurry (to reproduce the bug)"
    # Create a dummy file so the COPY in Dockerfile doesn't fail
    touch "$SCRIPT_DIR/hurry"
else
    info "Mode: Using LOCAL hurry (to test the fix)"

    # Step 1: Build hurry for Linux
    step "Building hurry for x86_64-unknown-linux-gnu"
    cargo cross build --release --target x86_64-unknown-linux-gnu -p hurry 2>&1 | tee "$OUTPUT_DIR/cross_build.log"

    HURRY_BINARY="$REPO_ROOT/target/x86_64-unknown-linux-gnu/release/hurry"
    if [[ ! -f "$HURRY_BINARY" ]]; then
        fail "Failed to build hurry binary at $HURRY_BINARY"
    fi
    info "Built hurry binary: $HURRY_BINARY"

    # Copy hurry binary to script directory (target/ is in .dockerignore)
    cp "$HURRY_BINARY" "$SCRIPT_DIR/hurry"
    info "Copied hurry binary to $SCRIPT_DIR/hurry"
fi

# Step 2: Run docker build
step "Running docker build (reproducing issue #319)"
echo "Logs will be saved to: $OUTPUT_DIR/docker_build.log"

# Use --progress=plain to get full output
# Use --platform linux/amd64 to match the user's environment
set +e  # Don't exit on error - we expect this to fail
docker buildx build \
    -t repro-319 \
    -f "$SCRIPT_DIR/Dockerfile" \
    --platform linux/amd64 \
    --secret "id=HURRY_API_TOKEN,env=HURRY_API_TOKEN" \
    --build-arg "USE_RELEASE=$USE_RELEASE" \
    --progress=plain \
    --no-cache \
    "$REPO_ROOT" 2>&1 | tee "$OUTPUT_DIR/docker_build.log"
BUILD_EXIT_CODE=${PIPESTATUS[0]}
set -e

# Step 3: Analyze output
step "Analyzing build output"
ANALYZE_LOG="$OUTPUT_DIR/docker_build.log"

# Extract fingerprint-related errors
echo "Extracting fingerprint errors..."
grep -E "dependency fingerprint|fingerprint hash not found" "$ANALYZE_LOG" > "$OUTPUT_DIR/fingerprint_errors.log" 2>/dev/null || true

# Extract unit missing messages
echo "Extracting cache miss information..."
grep -E "unit missing from cache|cache restore response" "$ANALYZE_LOG" > "$OUTPUT_DIR/cache_misses.log" 2>/dev/null || true

# Extract hurry restore activity
echo "Extracting hurry restore activity..."
grep -E "queuing unit restore|rewrite fingerprint|Restoring cache" "$ANALYZE_LOG" > "$OUTPUT_DIR/restore_activity.log" 2>/dev/null || true

# Extract incomplete deps filtering (the workaround)
echo "Extracting incomplete dependency filtering..."
grep -E "filtered units with incomplete dependency chains|filtering unit: incomplete dependency chain" "$ANALYZE_LOG" > "$OUTPUT_DIR/incomplete_deps_filter.log" 2>/dev/null || true

# Determine outcome
HAS_FINGERPRINT_ERROR=false
HAS_WORKAROUND=false
HAS_CACHE_RESPONSE=false
LOG_WAS_CLIPPED=false
WORKAROUND_COUNT=0

if grep -q "dependency fingerprint hash not found" "$ANALYZE_LOG"; then
    HAS_FINGERPRINT_ERROR=true
fi

if grep -q "filtered units with incomplete dependency chains" "$ANALYZE_LOG"; then
    HAS_WORKAROUND=true
    # Extract the count from the log message (format: "incomplete_deps_count: 27" or "incomplete_deps_count=27")
    WORKAROUND_COUNT=$(grep "filtered units with incomplete dependency chains" "$ANALYZE_LOG" | grep -oE 'incomplete_deps_count[=:] ?[0-9]+' | head -1 | grep -oE '[0-9]+' || echo "0")
fi

if grep -q "cache restore response" "$ANALYZE_LOG"; then
    HAS_CACHE_RESPONSE=true
fi

# Check if Docker clipped the output (important messages may be missing)
if grep -q "output clipped" "$ANALYZE_LOG"; then
    LOG_WAS_CLIPPED=true
fi

# Summary
echo ""
echo "=============================================="
echo "REPRODUCTION RESULT"
echo "=============================================="

if [[ $BUILD_EXIT_CODE -ne 0 ]]; then
    if [[ "$HAS_FINGERPRINT_ERROR" == "true" ]]; then
        warn "OUTCOME: BUG REPRODUCED"
        warn ""
        warn "Build FAILED with 'dependency fingerprint hash not found' error."
        warn "This confirms issue #319 is still present."
        OUTCOME="bug_reproduced"
    else
        warn "OUTCOME: BUILD FAILED (different error)"
        warn ""
        warn "Build failed (exit code: $BUILD_EXIT_CODE) but NOT with the expected error."
        warn "Check the logs for the actual failure reason."
        OUTCOME="other_failure"
    fi
elif [[ "$HAS_WORKAROUND" == "true" ]]; then
    info "OUTCOME: BUG FIXED (workaround active)"
    info ""
    info "Build SUCCEEDED because the incomplete dependency filter is working."
    info "Units with missing dependencies were skipped ($WORKAROUND_COUNT units filtered)."
    info "Cargo rebuilt those units from source."
    OUTCOME="fixed_with_workaround"
elif [[ "$LOG_WAS_CLIPPED" == "true" ]] && [[ "$HAS_CACHE_RESPONSE" == "false" ]]; then
    # Log was clipped and we didn't see the cache response message.
    # This likely means the workaround message was also clipped.
    # Since the build succeeded and we know the fix was applied, assume workaround is active.
    info "OUTCOME: BUILD SUCCEEDED (log clipped, likely workaround active)"
    info ""
    info "Build succeeded but Docker clipped the log output."
    info "The 'filtered units with incomplete dependency chains' message was likely clipped."
    info "Since the build succeeded and the fix is in place, the workaround is probably active."
    info ""
    warn "Note: To confirm, re-run with HURRY_LOG=info (debug produces too much output)"
    OUTCOME="fixed_with_workaround"
else
    info "OUTCOME: BUILD SUCCEEDED (no workaround needed)"
    info ""
    info "Build succeeded without needing the incomplete dependency filter."
    info "This could mean:"
    info "  - The cache now returns complete dependency chains"
    info "  - The glibc filtering is no longer excluding dependencies"
    info "  - Some other change resolved the underlying issue"
    OUTCOME="no_workaround_needed"
fi

echo "=============================================="
echo ""
echo "Output files:"
echo "  $OUTPUT_DIR/cross_build.log          - cargo cross build output"
echo "  $OUTPUT_DIR/docker_build.log         - Full docker build output"
echo "  $OUTPUT_DIR/fingerprint_errors.log   - Fingerprint-related errors"
echo "  $OUTPUT_DIR/cache_misses.log         - Units missing from cache"
echo "  $OUTPUT_DIR/restore_activity.log     - Hurry restore activity"
echo "  $OUTPUT_DIR/incomplete_deps_filter.log - Incomplete dependency filtering"
echo ""

# Show key findings
if [[ -s "$OUTPUT_DIR/cache_misses.log" ]]; then
    step "Cache response summary"
    grep "cache restore response" "$OUTPUT_DIR/cache_misses.log" | head -1 || true
    echo ""
fi

if [[ -s "$OUTPUT_DIR/incomplete_deps_filter.log" ]]; then
    step "Incomplete dependency filter summary"
    cat "$OUTPUT_DIR/incomplete_deps_filter.log" | head -10
    echo ""
fi

if [[ -s "$OUTPUT_DIR/fingerprint_errors.log" ]]; then
    step "Fingerprint errors"
    cat "$OUTPUT_DIR/fingerprint_errors.log" | head -5
    echo ""
fi

# Exit with appropriate code based on outcome
case "$OUTCOME" in
    "bug_reproduced")
        exit 1
        ;;
    "other_failure")
        exit 2
        ;;
    "fixed_with_workaround"|"no_workaround_needed")
        exit 0
        ;;
esac

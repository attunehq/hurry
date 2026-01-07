#!/usr/bin/env bash
set -euo pipefail

# Issue #319 Reproduction Script
# ==============================
# Reproduces the "dependency fingerprint hash not found" error that occurs
# when the cache returns a unit but not all of its dependencies.
#
# This script:
# 1. Clones the test project (github-rs at commit 3eeaef5b) into a Docker container
# 2. Runs hurry cargo build with the repro/319 org's cached artifacts
# 3. The cache returns incomplete dependency chains, triggering the bug
#
# Prerequisites:
#   - Docker with buildx support
#   - API token for accessing the `repro/319` org in Hurry production
#
# Usage:
#   export HURRY_API_TOKEN="<token-for-repro-319-org>"
#   ./packages/e2e/repros/319/repro.sh

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

fail() { echo -e "${RED}Error: $1${NC}" >&2; exit 1; }
info() { echo -e "${GREEN}$1${NC}" >&2; }
warn() { echo -e "${YELLOW}$1${NC}" >&2; }
step() { echo -e "${BLUE}==>${NC} $1" >&2; }

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

if ! command -v docker &> /dev/null; then
    fail "docker not found"
fi

if [[ -z "${HURRY_API_TOKEN:-}" ]]; then
    fail "HURRY_API_TOKEN environment variable not set.

This reproduction requires access to the 'repro/319' org in Hurry production.
Please obtain the API token for this org and set:

  export HURRY_API_TOKEN='<token-for-repro-319-org>'

Contact the Hurry team if you need access to this org."
fi

info "Using HURRY_API_URL=https://app.hurry.build/"

# Step 1: Run docker build
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
    --progress=plain \
    --no-cache \
    "$REPO_ROOT" 2>&1 | tee "$OUTPUT_DIR/docker_build.log"
BUILD_EXIT_CODE=${PIPESTATUS[0]}
set -e

# Step 2: Analyze output
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

# Summary
echo ""
echo "=============================================="
if [[ $BUILD_EXIT_CODE -eq 0 ]]; then
    info "Build SUCCEEDED (exit code: $BUILD_EXIT_CODE)"
    info "The bug may have been fixed!"
else
    warn "Build FAILED (exit code: $BUILD_EXIT_CODE)"

    # Check if it's the expected error
    if grep -q "dependency fingerprint hash not found" "$ANALYZE_LOG"; then
        info "Confirmed: This is the expected 'dependency fingerprint hash not found' error"
    else
        warn "Note: Build failed but with a different error than expected"
    fi
fi
echo "=============================================="
echo ""
echo "Output files:"
echo "  $OUTPUT_DIR/docker_build.log      - Full docker build output"
echo "  $OUTPUT_DIR/fingerprint_errors.log - Fingerprint-related errors"
echo "  $OUTPUT_DIR/cache_misses.log      - Units missing from cache"
echo "  $OUTPUT_DIR/restore_activity.log  - Hurry restore activity"
echo ""

# Show key findings
if [[ -s "$OUTPUT_DIR/cache_misses.log" ]]; then
    step "Cache response summary"
    grep "cache restore response" "$OUTPUT_DIR/cache_misses.log" | head -1 || true
    echo ""
    echo "Sample missing units:"
    grep "unit missing" "$OUTPUT_DIR/cache_misses.log" | head -5 || true
fi

exit $BUILD_EXIT_CODE

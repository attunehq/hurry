#!/usr/bin/env bash
set -euo pipefail

# Path Doubling Reproduction Script
# ==================================
# Reproduces the "could not find native static library ring_core_0_17_14_" error
# that occurs when hurry restores cached build artifacts in Docker.
#
# This script:
# 1. Builds hurry locally for Linux (using cargo cross)
# 2. Creates a debug Dockerfile that uses the local hurry binary
# 3. Runs docker compose build with CARGO_LOG fingerprint tracing
# 4. Outputs logs for analysis
#
# Prerequisites:
#   - cargo cross (cargo install cargo-cross)
#   - docker with buildx
#   - HURRY_API_TOKEN environment variable set
#
# Usage:
#   export HURRY_API_TOKEN="your-token"
#   ./packages/e2e/repros/295/repro.sh

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

if ! command -v cargo-cross &> /dev/null; then
    fail "cargo-cross not found. Install with: cargo install cargo-cross"
fi

if ! command -v docker &> /dev/null; then
    fail "docker not found"
fi

if [[ -z "${HURRY_API_TOKEN:-}" ]]; then
    fail "HURRY_API_TOKEN environment variable not set.

Please set it to your Hurry API token:
  export HURRY_API_TOKEN='your-token-here'

You can get this from your hurry dashboard or secrets manager."
fi

HURRY_API_URL="${HURRY_API_URL:-https://app.hurry.build/}"
info "Using HURRY_API_URL=$HURRY_API_URL"

# Step 1: Build hurry for Linux
step "Building hurry for x86_64-unknown-linux-gnu"
cargo cross build --release --target x86_64-unknown-linux-gnu -p hurry 2>&1 | tee "$OUTPUT_DIR/1_cross_build.log"

HURRY_BINARY="$REPO_ROOT/target/x86_64-unknown-linux-gnu/release/hurry"
if [[ ! -f "$HURRY_BINARY" ]]; then
    fail "Failed to build hurry binary at $HURRY_BINARY"
fi
info "Built hurry binary: $HURRY_BINARY"

# Step 2: Use debug Dockerfile from repro folder
step "Using debug Dockerfile"
DEBUG_DOCKERFILE="$SCRIPT_DIR/Dockerfile"
if [[ ! -f "$DEBUG_DOCKERFILE" ]]; then
    fail "Debug Dockerfile not found at $DEBUG_DOCKERFILE"
fi
info "Using $DEBUG_DOCKERFILE"

# Step 4: Run docker build with debug Dockerfile
step "Running docker build (this may take a while...)"
echo "Logs will be saved to: $OUTPUT_DIR/2_docker_build.log"

# Use --progress=plain to get full output
# Use --platform linux/amd64 to match CI environment
#
# Note: if you don't get the full log output, you may need to create a `buildx`
# context that does not truncate logs:
# ```
# docker buildx create --use --name limitless-logging --driver-opt env.BUILDKIT_STEP_LOG_MAX_SIZE=-1
# ```
# To go back to the default context, run:
# ```
# docker buildx use default --default
# ```
set +e  # Don't exit on error - we want to capture the failure
docker buildx build \
    -t courier-debug \
    -f "$DEBUG_DOCKERFILE" \
    --platform linux/amd64 \
    --secret "id=HURRY_API_TOKEN,env=HURRY_API_TOKEN" \
    --build-arg "HURRY_API_URL=$HURRY_API_URL" \
    --progress=plain \
    --no-cache \
    "$REPO_ROOT" 2>&1 | tee "$OUTPUT_DIR/2_docker_build.log"
BUILD_EXIT_CODE=${PIPESTATUS[0]}
set -e

# Step 5: Extract and analyze key information
step "Analyzing build output"
ANALYZE_LOG="$OUTPUT_DIR/2_docker_build.log"

# Extract fingerprint-related lines
echo "Extracting fingerprint analysis..."
grep -E "fingerprint|dirty|Dirty|stale|Stale|fresh|Fresh" "$ANALYZE_LOG" > "$OUTPUT_DIR/3_fingerprint_analysis.log" 2>/dev/null || true

# Extract ring-specific lines
echo "Extracting ring-related lines..."
grep -i "ring" "$ANALYZE_LOG" > "$OUTPUT_DIR/4_ring_related.log" 2>/dev/null || true

# Extract hurry restore lines
echo "Extracting hurry restore lines..."
grep -E "Restoring|restore|TRACE.*hurry|DEBUG.*hurry|write file|update mtime" "$ANALYZE_LOG" > "$OUTPUT_DIR/5_hurry_restore.log" 2>/dev/null || true

# Extract the error
echo "Extracting errors..."
grep -E "error|Error|ERROR|could not find" "$ANALYZE_LOG" > "$OUTPUT_DIR/6_errors.log" 2>/dev/null || true

# Extract hurry debug metadata (target directory tree)
# The metadata output starts with "=== HURRY DEBUG METADATA" and ends at "DONE".
# Lines are prefixed with docker step markers like "#26 0.357 ".
# We extract only the actual metadata lines (path -> Some(Metadata {...}))
echo "Extracting hurry debug metadata..."
awk '/=== HURRY DEBUG METADATA/,/^#[0-9]+ DONE/' "$ANALYZE_LOG" | grep -E ' -> (Some|None)' | sed 's/^#[0-9]* [0-9.]* //' > "$OUTPUT_DIR/7_target_metadata.log" 2>/dev/null || true

# Summary
echo ""
echo "=============================================="
if [[ $BUILD_EXIT_CODE -eq 0 ]]; then
    info "Build SUCCEEDED (exit code: $BUILD_EXIT_CODE)"
else
    warn "Build FAILED (exit code: $BUILD_EXIT_CODE)"
fi
echo "=============================================="
echo ""
echo "Output files:"
echo "  $OUTPUT_DIR/1_cross_build.log      - cargo cross build output"
echo "  $OUTPUT_DIR/2_docker_build.log     - full docker build output"
echo "  $OUTPUT_DIR/3_fingerprint_analysis.log - Cargo fingerprint/dirty analysis"
echo "  $OUTPUT_DIR/4_ring_related.log     - ring crate related lines"
echo "  $OUTPUT_DIR/5_hurry_restore.log    - hurry cache restore activity"
echo "  $OUTPUT_DIR/6_errors.log           - error messages"
echo "  $OUTPUT_DIR/7_target_metadata.log  - target directory tree (hurry debug metadata)"
echo ""

exit $BUILD_EXIT_CODE

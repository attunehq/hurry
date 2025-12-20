#!/usr/bin/env bash
set -uo pipefail

# Validation script for the path-doubling fix.
# Extracts logs and shows before/after comparison.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Use rg if available for highlighting, otherwise fall back to grep
if command -v rg &> /dev/null; then
    search() { rg --color=always "$@"; }
else
    search() { grep --color=always "$@"; }
fi

echo "=== Extracting logs ==="
tar -xzf logs.tar.gz
echo ""

echo "=== BEFORE: Doubled path in rustc invocation ==="
echo "(Note: look for the doubled path: /courier/target/release//courier/target/release/...)"
echo ""
search -B2 -A2 '\-L native=/courier/target/release//courier' run-local-repro/2_docker_build.log | head -10
echo ""

echo "=== BEFORE: Build error ==="
echo ""
search -A5 -B2 'could not find native static library' run-local-repro/2_docker_build.log || true
echo ""

echo "=== AFTER: Correct path in rustc invocation ==="
echo "(Note: look for the correct path: /courier/target/release/build/...)"
echo ""
search -B2 -A2 '\-L native=/courier/target/release/build/ring' run-local-fixed/2_docker_build.log | head -10
echo ""

echo "=== AFTER: Successful build ==="
echo ""
search 'Compiling courier|Finished' run-local-fixed/2_docker_build.log | tail -5
echo ""

echo "=== Cleaning up extracted logs ==="
rm -rf run-ci-1 run-ci-2 run-local-fixed run-local-repro
echo "Done."

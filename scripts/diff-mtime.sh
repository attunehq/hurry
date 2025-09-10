#!/usr/bin/env bash
#
# This helper script compares the restored Hurry cache with the Cargo cache and
# prints a diff.

set -euxo pipefail

# Check that rust-analyzer is not running.
if pgrep -f "rust-analyzer" > /dev/null; then
    echo "rust-analyzer is running. Please close it before running this script."
    exit 1
fi

# Run the `ready` script.
SCRIPT_DIR="$(dirname ${BASH_SOURCE[0]})"
$SCRIPT_DIR/ready.sh

# Create a run ID.
RUN_ID="$(date +%s)"
RUN_DIR="./.scratch/mtimes/$RUN_ID"
mkdir -p "$RUN_DIR"
echo "Saving run ID: $RUN_ID"

# After readying, we'll have just done a full clean build, so clean up the Cargo
# tree to remove first-party artifacts and then take a snapshot of the target
# mtimes in its current state.
rm -rf ./target/debug/{examples,hurry,hurry.d,incremental,libhurry.d,libhurry.rlib}
ls -lah --time-style=+%s.%N -R ./target > "$RUN_DIR/1-hurry-cargo-build.txt"

echo "Restoring build"
# Then, do a restored clean build and take a snapshot of the restored build.
cargo clean
hurry cargo build --hurry-skip-build --hurry-skip-backup
rm -rf ./target/debug/{examples,hurry,hurry.d,incremental,libhurry.d,libhurry.rlib}
ls -lah --time-style=+%s.%N -R ./target > "$RUN_DIR/2-restored-build.txt"
echo "Build restored"

# Save a mtime diff.
diff --side-by-side --width 200 "$RUN_DIR/1-hurry-cargo-build.txt" "$RUN_DIR/2-restored-build.txt" > "$RUN_DIR/diff.txt"

# # To view the latest mtime diff:
# less -S ./.scratch/mtimes/$(ls ./.scratch/mtimes | sort | tail -n 1)/diff.txt

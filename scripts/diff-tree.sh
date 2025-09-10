#!/usr/bin/env bash
#
# This helper script compares the restored Hurry cache with the Cargo cache and
# prints a diff.

set -euxo pipefail

# Run the `ready` script.
SCRIPT_DIR="$(dirname ${BASH_SOURCE[0]})"
$SCRIPT_DIR/ready.sh

# Create a run ID.
RUN_ID="$(date +%s)"
mkdir -p ./.scratch/trees/$RUN_ID
echo "Saving run ID: $RUN_ID"

# After this, we'll have just done a full clean build, so clean up the Cargo
# tree to remove first-party artifacts and then take a snapshot of the target
# tree in its current state.
rm -rf ./target/debug/{examples,hurry,hurry.d,incremental,libhurry.d,libhurry.rlib}
tree -a -f ./target/debug/ > ./.scratch/trees/$RUN_ID/cargo-tree.txt

echo "Restoring build"
# Then, do a restored clean build and take a snapshot of the restored build.
cargo clean
hurry cargo build --hurry-skip-build --hurry-skip-backup
rm -rf ./target/debug/{examples,hurry,hurry.d,incremental,libhurry.d,libhurry.rlib}
tree -a -f ./target/debug/ > ./.scratch/trees/$RUN_ID/hurry-tree.txt
echo "Build restored"

# Save a tree diff.
diff --side-by-side --width 200 ./.scratch/trees/$RUN_ID/cargo-tree.txt ./.scratch/trees/$RUN_ID/hurry-tree.txt > ./.scratch/trees/$RUN_ID/diff.txt

# # To view the latest tree diff:
# less -S ./.scratch/trees/$(ls ./.scratch/trees | sort | tail -n 1)/diff.txt

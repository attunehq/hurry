#!/usr/bin/env bash
#
# This helper script compares the restored Hurry cache with the Cargo cache and
# prints a diff. This is different from diff-tree because it can be run in any
# directory and creates diffs of `hurry cargo metadata`.

set -euxo pipefail

# Save the current directory.
WORK_DIR="$(pwd)"

# Do a clean installation of Hurry.
HURRY_DIR="$(dirname ${BASH_SOURCE[0]})"
HURRY_DIR="$(dirname $HURRY_DIR)"
cd $HURRY_DIR
cargo install --path ./packages/hurry --locked
cd $WORK_DIR
hurry cache reset --yes --remote "$@"

# Create a run ID.
RUN_ID="$(date +%s)"
mkdir -p $HURRY_DIR/.scratch/trees/$RUN_ID
echo "Saving run ID: $RUN_ID"

# Do a clean build and take a snapshot.
cargo clean
cargo build
hurry debug metadata ./target/debug/ > $HURRY_DIR/.scratch/trees/$RUN_ID/cargo-tree.txt

# Upload artifacts to Hurry.
hurry cargo build "$@"

# Do a restore and take a snapshot.
cargo clean
hurry cargo build --hurry-skip-build --hurry-skip-backup "$@"
hurry debug metadata ./target/debug/ > $HURRY_DIR/.scratch/trees/$RUN_ID/hurry-tree.txt

# Save a tree diff.
diff --side-by-side --width 200 $HURRY_DIR/.scratch/trees/$RUN_ID/cargo-tree.txt $HURRY_DIR/.scratch/trees/$RUN_ID/hurry-tree.txt > $HURRY_DIR/.scratch/trees/$RUN_ID/diff.txt

# # To view the latest tree diff:
# less -S ./.scratch/trees/$(ls ./.scratch/trees | sort | tail -n 1)/diff.txt

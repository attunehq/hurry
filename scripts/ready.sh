#!/usr/bin/env bash
#
# This helper script installs `hurry`, wipes the Cargo and Hurry caches, and
# warms the Hurry cache.

set -euxo pipefail

# TODO: For now, this is just a helper test script. We should turn this into an
# actual test suite at some point.

# Install test binary.
cargo install --path ./packages/hurry --locked

# Reset cache and build state.
cargo clean
hurry cache reset --yes

# Do a clean build and populate cache.
hurry cargo build

# Now we should be in a state to do a restored build.
echo "Ready"

# Some other useful commands:
#
# # Run restore-and-build:
# RUST_LOG=debug hurry cargo build --hurry-skip-backup --verbose 2>./.scratch/logs/$(date +%s).err
#
# # View latest logs:
# less -S ./.scratch/logs/$(ls ./.scratch/logs | sort | tail -n 1)

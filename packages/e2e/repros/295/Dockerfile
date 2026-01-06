# syntax=docker/dockerfile:1.4
#
# Minimal reproduction for issue #295: path doubling in build script output files.
#
# This Dockerfile reproduces the bug by:
#   1. Restoring cached build artifacts (including build script outputs) from Courier
#   2. Running cargo build, which reads the restored `root-output` file
#   3. The bug causes Cargo's path rewriting to double paths, breaking native library links
#
# Prerequisites:
#   1. Build hurry locally for linux:
#      cargo cross build --release --target x86_64-unknown-linux-gnu -p hurry
#
# To build (from repo root):
#   HURRY_API_TOKEN=xxx docker build -t repro-295 \
#     -f packages/e2e/repros/295/Dockerfile \
#     --platform linux/amd64 \
#     --secret id=HURRY_API_TOKEN,env=HURRY_API_TOKEN \
#     --build-arg HURRY_API_URL="https://app.hurry.build/" \
#     --progress=plain .
#
# Expected failure (before fix):
#   error: could not find native static library `ring_core_0_17_14_`
#   with doubled path: -L native=/courier/target/release//courier/target/release/...

FROM rust:1 AS builder
WORKDIR /courier

# Copy locally-built hurry binary. We do this instead of building hurry inside
# this container to reduce noise and confusion in the log messages.
COPY target/x86_64-unknown-linux-gnu/release/hurry /root/.local/bin/hurry
RUN chmod +x /root/.local/bin/hurry

# Config to connect to production Hurry.
ARG HURRY_API_URL
ENV HURRY_API_URL=${HURRY_API_URL:-}

# Enable additional debug logging for both hurry and cargo.
# - HURRY_LOG=debug: Shows hurry's restore operations and OUT_DIR file details
# - CARGO_LOG=...fingerprint=info: Shows dirty/stale decisions without verbose trace output
ENV HURRY_LOG=debug
ENV CARGO_LOG=cargo::core::compiler::fingerprint=info

COPY . .

# Restore cache and build - this triggers the path doubling bug
RUN --mount=type=secret,id=HURRY_API_TOKEN \
    HURRY_API_TOKEN=$(cat /run/secrets/HURRY_API_TOKEN) \
    ~/.local/bin/hurry cargo build --hurry-skip-backup --release --bin courier -v

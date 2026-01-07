# syntax=docker/dockerfile:1.4
#
# Reproduction for issue #319: dependency fingerprint hash not found
#
# This Dockerfile reproduces the bug by:
#   1. Setting up an x86_64 Linux environment matching the user's setup
#   2. Cloning the test project (github-rs)
#   3. Attempting to restore from a cache where some dependencies are missing
#   4. The bug causes fingerprint rewriting to fail due to missing dependency mappings
#
# Prerequisites:
#   1. Access to the `repro/319` org's API token in Hurry production
#
# To build (from repo root):
#   HURRY_API_TOKEN=xxx docker build -t repro-319 \
#     -f packages/e2e/repros/319/Dockerfile \
#     --platform linux/amd64 \
#     --secret id=HURRY_API_TOKEN,env=HURRY_API_TOKEN \
#     --progress=plain .
#
# Expected failure:
#   Error:
#      0: dependency fingerprint hash not found
#
#   Location:
#      packages/hurry/src/cargo/fingerprint.rs:221

FROM rust:1.92 AS builder
WORKDIR /workspace

# Install hurry from releases
RUN curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash
ENV PATH="/root/.local/bin:$PATH"

# Config to connect to Hurry production with the repro/319 org
ENV HURRY_API_URL=https://app.hurry.build/

# Enable debug logging
ENV HURRY_LOG=debug

# Clone the test project at the specific commit used for reproduction
RUN git clone https://github.com/yonasBSD/github-rs.git . && \
    git checkout 3eeaef5b27d4167f857ab6574065250066ba300e

# Attempt restore and build - this triggers the fingerprint hash not found bug
# Use --hurry-skip-backup to avoid polluting the cache state
# This step should FAIL with "dependency fingerprint hash not found" if the bug exists
RUN --mount=type=secret,id=HURRY_API_TOKEN \
    HURRY_API_TOKEN=$(cat /run/secrets/HURRY_API_TOKEN) \
    hurry cargo build --hurry-skip-backup --release

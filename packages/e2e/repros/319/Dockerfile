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
#   1. For local builds: cargo cross build --release --target x86_64-unknown-linux-gnu -p hurry
#   2. Access to the `repro/319` org (ID: 46) API token in Hurry production
#
# To build (from repo root):
#   HURRY_API_TOKEN=xxx docker build -t repro-319 \
#     -f packages/e2e/repros/319/Dockerfile \
#     --platform linux/amd64 \
#     --secret id=HURRY_API_TOKEN,env=HURRY_API_TOKEN \
#     --build-arg USE_RELEASE=false \
#     --progress=plain .
#
# Expected failure (without fix):
#   Error:
#      0: dependency fingerprint hash not found
#
#   Location:
#      packages/hurry/src/cargo/fingerprint.rs:221

FROM rust:1.92 AS builder
WORKDIR /workspace

# Build arg to control whether to use release or local hurry
# USE_RELEASE=true  -> install from releases (to reproduce the bug)
# USE_RELEASE=false -> use locally-built binary (to test the fix)
ARG USE_RELEASE=false

# Install hurry - either from releases or use locally-built binary
RUN if [ "$USE_RELEASE" = "true" ]; then \
        echo "Installing hurry from releases..." && \
        curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash; \
    else \
        echo "Using locally-built hurry binary..."; \
    fi

# Copy locally-built hurry binary (only used when USE_RELEASE=false)
# The binary is copied to packages/e2e/repros/319/ by repro.sh (target/ is in .dockerignore)
COPY packages/e2e/repros/319/hurry* /tmp/
RUN if [ "$USE_RELEASE" = "false" ]; then \
        mkdir -p /root/.local/bin && \
        cp /tmp/hurry /root/.local/bin/hurry && \
        chmod +x /root/.local/bin/hurry; \
    fi

ENV PATH="/root/.local/bin:$PATH"

# Config to connect to Hurry production with the repro/319 org
ENV HURRY_API_URL=https://app.hurry.build/

# Use warn logging to avoid clipping important messages (debug/info produce too much output)
# The workaround emits a WARN level message when units are filtered due to incomplete deps
ENV HURRY_LOG=warn

# Clone the test project at the specific commit used for reproduction
RUN git clone https://github.com/yonasBSD/github-rs.git . && \
    git checkout 3eeaef5b27d4167f857ab6574065250066ba300e

# Attempt restore and build - this triggers the fingerprint hash not found bug
# Use --hurry-skip-backup to avoid polluting the cache state
# This step should FAIL with "dependency fingerprint hash not found" if the bug exists
# Use pipefail so tee doesn't mask hurry's exit code
RUN --mount=type=secret,id=HURRY_API_TOKEN \
    set -o pipefail && \
    HURRY_API_TOKEN=$(cat /run/secrets/HURRY_API_TOKEN) \
    hurry cargo build --hurry-skip-backup --release 2>&1 | tee /tmp/hurry.log

# syntax=docker/dockerfile:1.4
#
# Debian container with Rust toolchain and hurry installed for e2e testing.
# Based on the official rust image which uses Debian.
#
# To build locally:
#   docker build -t hurry-e2e -f docker/hurry/Dockerfile \
#     --secret id=HURRY_API_TOKEN \
#     --build-arg HURRY_API_URL="$HURRY_API_URL" .
#
# To verify hurry caching works (force full rebuild, hurry should restore from remote):
#   docker build --no-cache -t hurry-e2e -f docker/hurry/Dockerfile \
#     --secret id=HURRY_API_TOKEN \
#     --build-arg HURRY_API_URL="$HURRY_API_URL" .

FROM rust:latest

# Install git for cloning test repos
RUN apt-get update && apt-get install -y \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install hurry (ADD fetches versions.json to bust cache when new version is released)
ADD https://hurry-releases.s3.amazonaws.com/releases/versions.json /tmp/hurry-versions.json
RUN curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash

# Hurry config (HURRY_API_TOKEN passed via --secret to avoid leaking into image layers)
ARG HURRY_API_URL
ENV HURRY_API_URL=${HURRY_API_URL:-}
ENV HURRY_WAIT_FOR_UPLOAD=true

# Copy hurry source code into the image
WORKDIR /hurry-src
COPY . .

# Install hurry binary from source using hurry for caching
RUN --mount=type=secret,id=HURRY_API_TOKEN \
    HURRY_API_TOKEN=$(cat /run/secrets/HURRY_API_TOKEN) \
    ~/.local/bin/hurry cargo install --path packages/hurry --force

# Set working directory for test execution
WORKDIR /workspace

# Keep container running for exec commands
CMD ["sleep", "infinity"]

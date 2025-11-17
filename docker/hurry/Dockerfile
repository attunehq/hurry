# syntax=docker/dockerfile:1.4
#
# Debian container with Rust toolchain and hurry installed for e2e testing.
# Based on the official rust image which uses Debian.

FROM rust:latest

# Install git for cloning test repos
RUN apt-get update && apt-get install -y \
    git \
    && rm -rf /var/lib/apt/lists/*

# Copy hurry source code into the image
WORKDIR /hurry-src
COPY . .

# Install hurry binary from source with cargo cache mount
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/hurry-src/target \
    cargo install --path packages/hurry --force

# Set working directory for test execution
WORKDIR /workspace

# Keep container running for exec commands
CMD ["sleep", "infinity"]

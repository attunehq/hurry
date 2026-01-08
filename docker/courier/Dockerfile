# syntax=docker/dockerfile:1.4
#
# Build the courier server with hurry caching.
#
# To build locally:
#   docker build -t courier -f docker/courier/Dockerfile \
#     --secret id=HURRY_API_TOKEN \
#     --build-arg HURRY_API_URL="$HURRY_API_URL" .
#
# To verify hurry caching works (force full rebuild, hurry should restore from remote):
#   docker build --no-cache -t courier -f docker/courier/Dockerfile \
#     --secret id=HURRY_API_TOKEN \
#     --build-arg HURRY_API_URL="$HURRY_API_URL" .

# Stage 1: Build the console (React SPA)
FROM node:20-alpine AS console-builder
WORKDIR /app
COPY packages/dashboard/package.json packages/dashboard/package-lock.json ./
RUN npm ci
COPY packages/dashboard/ ./
RUN npm run build

# Stage 2: Build courier with hurry caching
FROM rust:1 AS builder
WORKDIR /courier

# Install hurry (ADD fetches versions.json to bust cache when new version is released)
ADD https://hurry-releases.s3.amazonaws.com/releases/versions.json /tmp/hurry-versions.json
RUN curl -sSfL https://hurry-releases.s3.amazonaws.com/install.sh | bash

# Hurry config (HURRY_API_TOKEN passed via --secret to avoid leaking into image layers)
ARG HURRY_API_URL
ENV HURRY_API_URL=${HURRY_API_URL:-}
ENV HURRY_WAIT_FOR_UPLOAD=true

COPY . .
RUN --mount=type=secret,id=HURRY_API_TOKEN \
    HURRY_API_TOKEN=$(cat /run/secrets/HURRY_API_TOKEN) \
    ~/.local/bin/hurry cargo build --release --bin courier

# Stage 3: Runtime
FROM debian:bookworm-slim AS runtime
WORKDIR /courier
RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /courier/target/release/courier /usr/local/bin
COPY --from=console-builder /app/build/client /courier/console

USER 1000:1000
ENV RUST_LOG=courier=debug
ENV CONSOLE_DIR=/courier/console
HEALTHCHECK --interval=1s --timeout=5s CMD curl -f http://localhost:3000/api/v1/health

ENTRYPOINT ["/usr/local/bin/courier"]

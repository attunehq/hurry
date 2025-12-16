# TODO: Swap to hurry for caching!

# Stage 1: Build the console (React SPA)
FROM node:20-alpine AS console-builder
WORKDIR /app
COPY packages/dashboard/package.json packages/dashboard/package-lock.json ./
RUN npm ci
COPY packages/dashboard/ ./
RUN npm run build

# Stage 2: Prepare Rust dependencies
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /courier

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /courier/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin courier

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

# TODO: Swap to hurry for caching!

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

FROM debian:bookworm-slim AS runtime
WORKDIR /courier
RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /courier/target/release/courier /usr/local/bin

USER 1000:1000
ENV RUST_LOG=courier=debug
HEALTHCHECK --interval=1s --timeout=5s CMD curl -f http://localhost:3000/api/v1/health

ENTRYPOINT ["/usr/local/bin/courier"]

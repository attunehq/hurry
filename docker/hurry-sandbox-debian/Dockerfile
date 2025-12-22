# This Dockerfile is used to create a sandbox container for testing
# cross-machine build save/restores with Hurry. It is capable of building Hurry
# and some other test Rust projects.
#
# To build:
#
# ```
# docker build -t hurry-sandbox-debian -f ./docker/hurry-sandbox-debian/Dockerfile .
# ```
#
# To run:
#
# ```
# docker run --name=hurry-sandbox-$(date +%s)
#  --interactive \
#  --tty \
#  --network=host \
#  --volume ~/src/attune/attune:/root/src/attune \
#  --volume ~/src/attune/hurry:/root/src/hurry \
#  hurry-sandbox-debian:latest
# ```
#
# To resume a stopped but existing container, use:
# ```
# docker start --interactive --attach hurry-sandbox-$SANDBOX_ID
# ```

FROM debian:bookworm-20251020

RUN apt update && apt install -y curl git git-lfs build-essential pkg-config libssl-dev libgpg-error-dev libgpgme-dev file vim jq
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

WORKDIR /root

COPY . /root/src/hurry
RUN cd ./src/hurry && . ~/.cargo/env && cargo install --locked --path ./packages/hurry

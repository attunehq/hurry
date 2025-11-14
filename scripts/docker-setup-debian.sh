# docker run -it --network=host --name=hurry-sandbox-$(date +%s) \
#  -v /tmp/hurry/cache:/tmp/hurry/cache \
#  -v $ATTUNE_REPO:/root/src/attune \
#  -v $HURRY_REPO:/root/src/hurry \
#  debian:bookworm-20251020 /bin/bash
apt update
apt install -y curl build-essential git pkg-config libssl-dev libgpg-error-dev libgpgme-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cd
source .profile
cd src/hurry
cargo install --path ./packages/hurry --locked

#!/usr/bin/env bash
# One-time setup on Ubuntu 24.04+ (VPS). Needs sudo for the apt step.
set -euo pipefail
sudo apt-get update
sudo apt-get install -y build-essential curl ffmpeg libfontconfig1-dev libfreetype-dev fontconfig
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component rustfmt,clippy
  . "$HOME/.cargo/env"
fi
cd "$(dirname "$0")/.."
cargo build --release
echo "Built: $(pwd)/target/release/nova"
./target/release/nova doctor

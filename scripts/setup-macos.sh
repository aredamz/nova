#!/usr/bin/env bash
# One-time setup on macOS (Apple Silicon or Intel). Requires Homebrew: https://brew.sh
set -euo pipefail
command -v brew >/dev/null || { echo "Homebrew is required (https://brew.sh)"; exit 1; }
# ffmpeg: encoding (includes libx264 and ffprobe). fontconfig: `fc-match`, used by the
# Remotion bake tools to find system fonts.
brew install ffmpeg fontconfig
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi
cd "$(dirname "$0")/.."
cargo build --release
echo "Built: $(pwd)/target/release/nova"
./target/release/nova doctor

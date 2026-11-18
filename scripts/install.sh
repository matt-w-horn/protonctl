#!/bin/sh
# Build protonctl from this clone, sign it, and install it into ~/.cargo/bin
# scripts/sign.sh does the signing and says what its prompts mean.
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    echo "install.sh signs for the macOS Keychain; on Linux run: cargo install --path . --locked" >&2
    exit 1
fi

cargo build --release --locked
scripts/sign.sh target/release/protonctl "$HOME/.cargo/bin/protonctl"

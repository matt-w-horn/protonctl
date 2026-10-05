#!/bin/sh
# Every local gate (RFC section 3, Gates), in one place. The pre-commit hook
# runs it on each commit; run it by hand to see the same verdict.
set -eu
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check
scripts/plugin-check.sh

# On Linux, also lint the macOS build (RFC section 11), so the macOS platform
# file cannot drift unseen. Nothing built here is linked or run: ring's C code
# compiles freestanding, against a stub of the one Apple header it includes.
if [ "$(uname -s)" = Linux ]; then
    if command -v clang >/dev/null &&
        rustup target list --installed 2>/dev/null | grep -qx aarch64-apple-darwin; then
        CC_aarch64_apple_darwin=clang \
            CFLAGS_aarch64_apple_darwin="--target=aarch64-apple-macos11 -ffreestanding -nostdinc \
                -isystem $(clang -print-resource-dir)/include -isystem $PWD/scripts/apple-stub \
                -DRING_CORE_NOSTDLIBINC" \
            cargo clippy --target aarch64-apple-darwin --all-targets --locked -- -D warnings
    else
        echo "check.sh: skipped the macOS build check (needs clang and \`rustup target add aarch64-apple-darwin\`)" >&2
    fi
    # The Secret Service store (RFC Q31), against a throwaway GNOME Keyring in
    # a private D-Bus session, so the user's own keyring is never touched.
    # XDG_DATA_HOME covers the whole session, so a keyring the bus starts on
    # its own uses the throwaway folder too; and the tests wait until the
    # keyring owns its name, which it does a moment after the daemon returns.
    if command -v dbus-run-session >/dev/null && command -v gnome-keyring-daemon >/dev/null; then
        keyring=$(mktemp -d)
        trap 'rm -rf "$keyring"' EXIT
        XDG_DATA_HOME="$keyring" dbus-run-session -- sh -c '
            printf test | gnome-keyring-daemon --unlock --components=secrets >/dev/null || exit 1
            tries=0
            until dbus-send --session --print-reply --dest=org.freedesktop.DBus / \
                org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets |
                grep -q "boolean true"; do
                tries=$((tries + 1))
                if [ "$tries" -gt 50 ]; then
                    echo "check.sh: the throwaway keyring did not start" >&2
                    exit 1
                fi
                sleep 0.1
            done
            cargo test --locked --bin protonctl platform::linux -- --ignored'
    else
        echo "check.sh: skipped the Secret Service tests (needs dbus-run-session and gnome-keyring-daemon)" >&2
    fi
fi

# Line coverage may not fall below the floor: the 2026-10-03 figure, 62.99%,
# rounded down. Raise it as tests grow. On a Mac, Homebrew's llvm matches its
# rustc; elsewhere cargo-llvm-cov uses rustup's llvm-tools-preview component.
floor=62
brew_llvm=/opt/homebrew/opt/llvm/bin
# Each one set by hand is kept; only an unset one falls back to Homebrew's.
if [ -x "$brew_llvm/llvm-cov" ]; then
    export LLVM_COV="${LLVM_COV:-$brew_llvm/llvm-cov}" \
        LLVM_PROFDATA="${LLVM_PROFDATA:-$brew_llvm/llvm-profdata}"
fi
if ! cargo llvm-cov --locked --summary-only --fail-under-lines "$floor"; then
    echo "check.sh: line coverage is below $floor% (or cargo-llvm-cov is missing)" >&2
    exit 1
fi
echo "check.sh: all gates passed"

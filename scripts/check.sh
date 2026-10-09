#!/bin/sh
# Every local gate (RFC section 3, Gates), in one place. The pre-commit hook
# runs it on each commit; run it by hand to see the same verdict.
set -eu
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo deny check
scripts/plugin-check.sh

# The tests run once, instrumented, and the coverage report at the end reads
# their profiles: one full build of the tree, not a plain one and an
# instrumented one, since the second build was what filled CI's disk once
# the name model's crates joined (2026-10-08). Each `--no-report` run below
# adds to the same profiles. On a Mac, Homebrew's llvm matches its rustc;
# elsewhere cargo-llvm-cov uses rustup's llvm-tools-preview component.
brew_llvm=/opt/homebrew/opt/llvm/bin
# Each one set by hand is kept; only an unset one falls back to Homebrew's.
if [ -x "$brew_llvm/llvm-cov" ]; then
    export LLVM_COV="${LLVM_COV:-$brew_llvm/llvm-cov}" \
        LLVM_PROFDATA="${LLVM_PROFDATA:-$brew_llvm/llvm-profdata}"
fi
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report --locked

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
    if command -v dbus-run-session >/dev/null && command -v gnome-keyring-daemon >/dev/null; then
        keyring=$(mktemp -d)
        trap 'rm -rf "$keyring"' EXIT
        XDG_DATA_HOME="$keyring" XDG_RUNTIME_DIR="$keyring" dbus-run-session -- \
            scripts/with-keyring.sh cargo llvm-cov test --no-report --locked --bin protonctl platform::linux -- --ignored
        # A Drive read through the readers, with the CLI's pin in the store (Q33).
        XDG_DATA_HOME="$keyring" XDG_RUNTIME_DIR="$keyring" dbus-run-session -- \
            scripts/with-keyring.sh cargo llvm-cov test --no-report --locked --test convert -- --ignored
    else
        echo "check.sh: skipped the Secret Service tests (needs dbus-run-session and gnome-keyring-daemon)" >&2
    fi
    # The hermetic IMAP test (RFC section 7, T11): every mail read against
    # Dovecot in a throwaway container.
    if command -v podman >/dev/null; then
        scripts/with-dovecot.sh
    else
        echo "check.sh: skipped the Dovecot test (needs podman)" >&2
    fi
fi

# Line coverage may not fall below the floor: the 2026-10-03 figure, 62.99%,
# rounded down. Raise it as tests grow. The report merges every run above.
floor=62
if ! cargo llvm-cov report --summary-only --fail-under-lines "$floor"; then
    echo "check.sh: line coverage is below $floor% (or cargo-llvm-cov is missing)" >&2
    exit 1
fi
echo "check.sh: all gates passed"

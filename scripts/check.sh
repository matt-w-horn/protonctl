#!/bin/sh
# Every local gate (RFC section 3, Gates), in one place. The pre-commit hook
# runs it on each commit; run it by hand to see the same verdict.
set -eu
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check

# Line coverage may not fall below the floor: the 2026-10-03 figure, 62.99%,
# rounded down. Raise it as tests grow. Homebrew's llvm matches its rustc.
floor=62
if ! LLVM_COV="${LLVM_COV:-/opt/homebrew/opt/llvm/bin/llvm-cov}" \
    LLVM_PROFDATA="${LLVM_PROFDATA:-/opt/homebrew/opt/llvm/bin/llvm-profdata}" \
    cargo llvm-cov --locked --summary-only --fail-under-lines "$floor"; then
    echo "check.sh: line coverage is below $floor% (or cargo-llvm-cov is missing)" >&2
    exit 1
fi
echo "check.sh: all gates passed"

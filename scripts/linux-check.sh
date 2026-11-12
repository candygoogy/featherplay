#!/usr/bin/env bash
# Checks that the workspace builds for Linux: `cargo check`, then clippy with warnings as
# errors, for x86_64-unknown-linux-gnu. Run it on Linux or inside WSL, from anywhere in the
# repository:
#   scripts/linux-check.sh
#
# It needs the target's standard library (`rustup target add x86_64-unknown-linux-gnu`, which
# a Linux toolchain has already). A quick check before a push; CI's `linux` job runs clippy
# and the tests on Ubuntu (.github/workflows/ci.yml).
set -euo pipefail

cd "$(dirname "$0")/.."

target=x86_64-unknown-linux-gnu

echo "cargo check ($target)"
cargo check --workspace --all-targets --target "$target"

echo "cargo clippy ($target)"
cargo clippy --workspace --all-targets --target "$target" -- -D warnings

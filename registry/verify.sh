#!/usr/bin/env sh
# Builds the verifier with the pinned toolchain and dependencies, then runs it.
# All arguments go to the verifier. Run ./verify.sh -h for its options.
set -eu
cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is not installed. Install Rust with rustup (https://rustup.rs), then run this script again." >&2
    exit 2
fi
if ! command -v rustup >/dev/null 2>&1; then
    echo "warning: rustup is not installed, so rust-toolchain.toml cannot pin the compiler version." >&2
fi

echo "Building the verifier (the first build takes several minutes)..."
cargo build --release --locked --quiet
exec ./target/release/dotk-verify "$@"

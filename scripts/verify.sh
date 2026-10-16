#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all -- --check
cargo test --workspace --locked
node --test scripts/test-vpn-platform.mjs
scripts/verify-local-protocols.sh
scripts/ohrs-build.sh --arch aarch --package paws_ui -- --locked

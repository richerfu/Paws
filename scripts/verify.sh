#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo test --workspace
node --test scripts/test-vpn-platform.mjs
scripts/verify-local-protocols.sh
scripts/ohrs-build.sh --arch aarch

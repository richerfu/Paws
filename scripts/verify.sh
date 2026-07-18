#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo test --workspace
scripts/verify-local-protocols.sh
ohrs build --arch aarch

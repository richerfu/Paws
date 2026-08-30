#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo test --workspace
scripts/verify-local-protocols.sh
scripts/ohrs-build.sh --arch aarch

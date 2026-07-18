#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
KEEPALIVE_MS="${KEEPALIVE_MS:-100}"
MODES="${MODES:-direct http http-auth http-bad-auth http-down socks5 socks5-auth socks5-bad-auth ss ss-bad-password trojan trojan-bad-password vless vless-bad-uuid}"

for mode in $MODES; do
  echo "Verifying local protocol profile: $mode"
  (
    cd "$ROOT_DIR"
    cargo run --manifest-path local-protocol-tests/Cargo.toml -- "$mode" --keepalive-ms "$KEEPALIVE_MS"
  )
done

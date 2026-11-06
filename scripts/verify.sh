#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all -- --check
cargo test --workspace --locked
node --test scripts/test-vpn-platform.mjs
python3 scripts/test-benchmark-harmony-vpn.py
python3 scripts/test-socket-evidence.py
python3 scripts/test-write-build-manifest.py
python3 scripts/test-http-obfs-compat.py --run 'TestFirstRequest|TestFragmentedResponse/sing-box-compatible|TestRoundTrip/sing-box-compatible|TestFirstFlightAEADRecovery|TestCapturedWireForensicsSynthetic' --output dist/TinyPaws-http-obfs-compatible-local.json
go test -race -count=1 -timeout=30s vendor/sing-box-bridge/tlsretry/conn.go vendor/sing-box-bridge/tlsretry/conn_test.go
go test vendor/sing-box-bridge/physical_dns.go vendor/sing-box-bridge/physical_dns_test.go
sh scripts/test-sing-box-traffic.sh
scripts/verify-local-protocols.sh
scripts/ohrs-build.sh --arch aarch --package paws_ui -- --locked

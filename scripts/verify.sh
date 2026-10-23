#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all -- --check
cargo test --workspace --locked
python3 scripts/check-architecture.py
python3 scripts/test-nsh-build-inputs.py
python3 scripts/test-nsh-build-source.py
python3 scripts/test-signing-profile.py
python3 scripts/test-sign-existing-hap.py
python3 ui/nsh/run-tests.py
python3 ui/nsh/tests/run-about-tests.py
python3 ui/nsh/tests/run-activity-tests.py
python3 ui/nsh/tests/run-refresh-tests.py
python3 ui/nsh/tests/run-visual-tests.py
python3 ui/nsh/tests/run-page-error-tests.py
python3 ui/nsh/tests/run-engine-preview-tests.py
python3 ui/nsh/tests/run-settings-mtu-tests.py
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

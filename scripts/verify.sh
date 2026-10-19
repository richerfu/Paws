#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo test --workspace
node --test scripts/test-vpn-platform.mjs
python3 scripts/test-benchmark-harmony-vpn.py
python3 scripts/test-socket-evidence.py
python3 scripts/test-write-build-manifest.py
python3 scripts/test-http-obfs-compat.py --run 'TestFirstRequest|TestFragmentedResponse/sing-box-compatible|TestRoundTrip/sing-box-compatible|TestFirstFlightAEADRecovery|TestCapturedWireForensicsSynthetic' --output dist/TinyPaws-http-obfs-compatible-local.json
go test -race -count=1 -timeout=30s vendor/sing-box-bridge/tlsretry/conn.go vendor/sing-box-bridge/tlsretry/conn_test.go
go test vendor/sing-box-bridge/physical_dns.go vendor/sing-box-bridge/physical_dns_test.go
scripts/verify-local-protocols.sh
if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk/default/openharmony ]; then
  OHOS_NDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony
  export OHOS_NDK_HOME
fi
if [ -z "${DEVECO_SDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk ]; then
  DEVECO_SDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk
  export DEVECO_SDK_HOME
fi
scripts/ohrs-build.sh --arch aarch

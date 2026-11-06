#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_PARENT="$(cd "$ROOT_DIR/.." && pwd)"
SING_BOX_ROOT="${SING_BOX_ROOT:-$WORKSPACE_PARENT/sing-box-ohos}"
GO_OHOS_ROOT="${GO_OHOS_ROOT:-$WORKSPACE_PARENT/go-ohos-1.24.7-compat}"
OHOS_NDK_HOME="${OHOS_NDK_HOME:-/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony}"
GO_COMPILER_ROOT="$(sh "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$GO_OHOS_ROOT")"
GO_BIN="$GO_COMPILER_ROOT/bin/go"
CC_BIN="$OHOS_NDK_HOME/native/llvm/bin/aarch64-unknown-linux-ohos-clang"
OUTPUT="${SING_BOX_OUTPUT:-$ROOT_DIR/entry/libs/arm64-v8a/libsingbox.so}"

test -x "$GO_BIN" || { echo "OpenHarmony Go compiler not found: $GO_BIN" >&2; exit 1; }
test -x "$CC_BIN" || { echo "OpenHarmony clang not found: $CC_BIN" >&2; exit 1; }
test "$(git -C "$SING_BOX_ROOT" rev-parse HEAD)" = "628cb31ffa79cffffd34c2f9cde6cae044e4fc12" || {
  echo "sing-box v1.13.21 source revision mismatch" >&2
  exit 1
}

if ! grep -Fq RequiresEagerShadowsocksHandshake "$SING_BOX_ROOT/protocol/shadowsocks/outbound.go"; then
  git -C "$SING_BOX_ROOT" apply --check "$ROOT_DIR/patches/sing-box-http-obfs-handshake.patch"
  git -C "$SING_BOX_ROOT" apply "$ROOT_DIR/patches/sing-box-http-obfs-handshake.patch"
fi
if ! grep -Fq DialHandshakeContext "$SING_BOX_ROOT/protocol/shadowsocks/outbound.go"; then
  git -C "$SING_BOX_ROOT" apply --check "$ROOT_DIR/patches/sing-box-http-obfs-first-flight.patch"
  git -C "$SING_BOX_ROOT" apply "$ROOT_DIR/patches/sing-box-http-obfs-first-flight.patch"
fi
if ! grep -Fq newTinyProbeConn "$SING_BOX_ROOT/protocol/mixed/inbound.go"; then
  git -C "$SING_BOX_ROOT" apply --check "$ROOT_DIR/patches/sing-box-benchmark-probe.patch"
  git -C "$SING_BOX_ROOT" apply "$ROOT_DIR/patches/sing-box-benchmark-probe.patch"
fi
mkdir -p "$SING_BOX_ROOT/cmd/libtinypaws/tlsretry" "$SING_BOX_ROOT/cmd/libtinypaws/httpobfs" "$(dirname "$OUTPUT")"
cp "$ROOT_DIR/vendor/sing-box-bridge/main.go" "$SING_BOX_ROOT/cmd/libtinypaws/main.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/obfs_plugin.go" "$SING_BOX_ROOT/cmd/libtinypaws/obfs_plugin.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/physical_dns.go" "$SING_BOX_ROOT/cmd/libtinypaws/physical_dns.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/physical_network.go" "$SING_BOX_ROOT/cmd/libtinypaws/physical_network.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/tlsretry/conn.go" "$SING_BOX_ROOT/cmd/libtinypaws/tlsretry/conn.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/trace.go" "$SING_BOX_ROOT/cmd/libtinypaws/trace.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/traffic.go" "$SING_BOX_ROOT/cmd/libtinypaws/traffic.go"
cp "$ROOT_DIR/vendor/sing-box-bridge/httpobfs/conn.go" "$SING_BOX_ROOT/cmd/libtinypaws/httpobfs/conn.go"

cd "$SING_BOX_ROOT"
env GOTOOLCHAIN=local GOOS=openharmony GOARCH=arm64 CGO_ENABLED=1 CC="$CC_BIN" \
  "$GO_BIN" build -buildmode=c-shared \
  -tags 'with_gvisor,with_quic,with_dhcp,with_utls,with_clash_api' \
  -ldflags '-s -w' -o "$OUTPUT" ./cmd/libtinypaws

file "$OUTPUT"
shasum -a 256 "$OUTPUT"

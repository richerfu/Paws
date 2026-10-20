#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SING_BOX_ROOT=${SING_BOX_ROOT:-$ROOT_DIR/target/engine-sources/sing-box-ohos}
test -f "$SING_BOX_ROOT/go.mod" || {
  echo "Set SING_BOX_ROOT to the pinned sing-box checkout (see docs/native-engines-build.md)." >&2
  exit 1
}
cd "$SING_BOX_ROOT"
go test -race "$ROOT_DIR/vendor/sing-box-bridge/traffic.go" \
  "$ROOT_DIR/vendor/sing-box-bridge/traffic_test.go"

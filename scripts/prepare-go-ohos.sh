#!/usr/bin/env sh
set -eu
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
exec python3 "$ROOT_DIR/scripts/go_toolchain.py" "${1:-${GO_OHOS_ROOT:-}}"

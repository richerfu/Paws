#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$ROOT_DIR/scripts/ohos-env.sh"
BUILD_TARGET_ROOT="${PAWS_NATIVE_TARGET_DIR:-$ROOT_DIR/target}"
case "$BUILD_TARGET_ROOT" in /*) ;; *) BUILD_TARGET_ROOT="$ROOT_DIR/$BUILD_TARGET_ROOT" ;; esac
export CARGO_TARGET_DIR="$BUILD_TARGET_ROOT"
export PAWS_NSH_REVISION="${PAWS_NSH_REVISION:-$(python3 "$ROOT_DIR/scripts/nsh_build_inputs.py" --field nshRevision)}"
if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk/default/openharmony ]; then
  export OHOS_NDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony
fi
if [ -z "${DEVECO_SDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk ]; then
  export DEVECO_SDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk
fi
test -d "${OHOS_NDK_HOME:-}/native" || { echo "OpenHarmony NDK is unavailable" >&2; exit 1; }
cd "$ROOT_DIR"
OHRS_BIN=${OHRS:-ohrs}
exec "$OHRS_BIN" build "$@"

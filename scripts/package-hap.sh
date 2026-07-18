#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OHRS="${OHRS:-ohrs}"
ARCH="${ARCH:-aarch}"
SO_SRC="$ROOT_DIR/target/aarch64-unknown-linux-ohos/debug/libhmeta_ui.so"
SO_DST="$ROOT_DIR/entry/libs/arm64-v8a/libhmeta_ui.so"
HAP_PATH="${HAP_PATH:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-unsigned.hap}"
HVIGOR_ARGS="${HVIGOR_ARGS:---no-daemon}"

if [ -n "${HVIGORW:-}" ]; then
  HVIGORW_BIN="$HVIGORW"
elif [ -x "$ROOT_DIR/hvigorw" ]; then
  HVIGORW_BIN="$ROOT_DIR/hvigorw"
elif [ -x "/Applications/DevEco-Studio.app/Contents/tools/hvigor/bin/hvigorw" ]; then
  HVIGORW_BIN="/Applications/DevEco-Studio.app/Contents/tools/hvigor/bin/hvigorw"
else
  HVIGORW_BIN="$(command -v hvigorw)"
fi

if [ -z "${DEVECO_SDK_HOME:-}" ] && [ -d "/Applications/DevEco-Studio.app/Contents/sdk" ]; then
  export DEVECO_SDK_HOME="/Applications/DevEco-Studio.app/Contents/sdk"
fi

if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d "/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony" ]; then
  export OHOS_NDK_HOME="/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony"
fi

cd "$ROOT_DIR"
"$OHRS" build --arch "$ARCH"
cp "$SO_SRC" "$SO_DST"
"$HVIGORW_BIN" default@PackageHap --mode module -p module=entry@default $HVIGOR_ARGS

if [ ! -f "$HAP_PATH" ]; then
  echo "Expected unsigned HAP was not generated: $HAP_PATH" >&2
  exit 1
fi

echo "$HAP_PATH"

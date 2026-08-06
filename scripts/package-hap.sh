#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEVECO_STUDIO_HOME="${DEVECO_STUDIO_HOME:-/Applications/DevEco-Studio.app/Contents}"
OHRS="${OHRS:-ohrs}"
ARCH="${ARCH:-aarch}"
NATIVE_PROFILE="${NATIVE_PROFILE:-release}"
HAP_BUILD_MODE="${HAP_BUILD_MODE:-release}"
case "$NATIVE_PROFILE" in
  release)
    SO_SRC="$ROOT_DIR/target/aarch64-unknown-linux-ohos/release/libhmeta_ui.so"
    ;;
  debug)
    SO_SRC="$ROOT_DIR/target/aarch64-unknown-linux-ohos/debug/libhmeta_ui.so"
    ;;
  *)
    echo "Unsupported NATIVE_PROFILE: $NATIVE_PROFILE (expected release or debug)" >&2
    exit 1
    ;;
esac
case "$HAP_BUILD_MODE" in
  release|debug)
    ;;
  *)
    echo "Unsupported HAP_BUILD_MODE: $HAP_BUILD_MODE (expected release or debug)" >&2
    exit 1
    ;;
esac
SO_DST="$ROOT_DIR/entry/libs/arm64-v8a/libhmeta_ui.so"
HAP_PATH="${HAP_PATH:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-unsigned.hap}"
HVIGOR_ARGS="${HVIGOR_ARGS:---no-daemon}"

if [ -n "${HVIGORW:-}" ]; then
  HVIGORW_BIN="$HVIGORW"
elif [ -x "$ROOT_DIR/hvigorw" ]; then
  HVIGORW_BIN="$ROOT_DIR/hvigorw"
elif [ -x "$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw" ]; then
  HVIGORW_BIN="$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw"
else
  HVIGORW_BIN="$(command -v hvigorw)"
fi

# Keep Hvigor and DevEco on the same SDK even when the shell exports an older
# command-line SDK via DEVECO_SDK_HOME.
if [ -d "$DEVECO_STUDIO_HOME/sdk" ]; then
  export DEVECO_SDK_HOME="$DEVECO_STUDIO_HOME/sdk"
fi

if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d "$DEVECO_STUDIO_HOME/sdk/default/openharmony" ]; then
  export OHOS_NDK_HOME="$DEVECO_STUDIO_HOME/sdk/default/openharmony"
fi
if [ -n "${DEVECO_NODE_HOME:-}" ]; then
  export NODE_HOME="$DEVECO_NODE_HOME"
elif [ -x "$DEVECO_STUDIO_HOME/tools/node/bin/node" ]; then
  export NODE_HOME="$DEVECO_STUDIO_HOME/tools/node"
fi
if [ -n "${NODE_HOME:-}" ]; then
  export PATH="$NODE_HOME/bin:$PATH"
fi

cd "$ROOT_DIR"
if [ "$NATIVE_PROFILE" = "release" ]; then
  "$OHRS" build --arch "$ARCH" --release
else
  "$OHRS" build --arch "$ARCH"
fi
cp "$SO_SRC" "$SO_DST"
"$HVIGORW_BIN" default@PackageHap --mode module -p module=entry@default \
  -p buildMode="$HAP_BUILD_MODE" $HVIGOR_ARGS

if [ ! -f "$HAP_PATH" ]; then
  echo "Expected unsigned HAP was not generated: $HAP_PATH" >&2
  exit 1
fi

echo "$HAP_PATH"

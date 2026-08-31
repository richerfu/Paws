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
    SO_SRC="$ROOT_DIR/target/aarch64-unknown-linux-ohos/release/libpaws_ui.so"
    ;;
  debug)
    SO_SRC="$ROOT_DIR/target/aarch64-unknown-linux-ohos/debug/libpaws_ui.so"
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
NATIVE_LIB_DIR="$ROOT_DIR/entry/libs/arm64-v8a"
SO_DST="$NATIVE_LIB_DIR/libpaws_ui.so"
CXX_SHARED_DST="$NATIVE_LIB_DIR/libc++_shared.so"
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
if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d "${DEVECO_SDK_HOME:-}/default/openharmony" ]; then
  export OHOS_NDK_HOME="$DEVECO_SDK_HOME/default/openharmony"
fi
if [ -z "${CXX_SHARED_SRC:-}" ]; then
  for cxx_runtime_candidate in \
    "${OHOS_NDK_HOME:-}/native/llvm/lib/aarch64-linux-ohos/libc++_shared.so" \
    "${OHOS_NDK_HOME:-}/native/llvm/lib/aarch64-linux-ohos/c++/libc++_shared.so"
  do
    if [ -f "$cxx_runtime_candidate" ]; then
      CXX_SHARED_SRC="$cxx_runtime_candidate"
      break
    fi
  done
fi
if [ ! -f "${CXX_SHARED_SRC:-}" ]; then
  echo "HarmonyOS libc++_shared.so was not found under ${OHOS_NDK_HOME:-<unset>}" >&2
  echo "Set OHOS_NDK_HOME or CXX_SHARED_SRC to the active HarmonyOS arm64 runtime." >&2
  exit 1
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
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH" --release
else
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH"
fi
cp "$SO_SRC" "$SO_DST"
cp "$CXX_SHARED_SRC" "$CXX_SHARED_DST"
"$HVIGORW_BIN" default@PackageHap --mode module -p module=entry@default \
  -p buildMode="$HAP_BUILD_MODE" $HVIGOR_ARGS

if [ ! -f "$HAP_PATH" ]; then
  echo "Expected unsigned HAP was not generated: $HAP_PATH" >&2
  exit 1
fi

echo "$HAP_PATH"

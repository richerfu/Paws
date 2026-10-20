#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

SIGNED=0
case "${1:-}" in
  --signed) SIGNED=1; shift ;;
  -h|--help)
    cat <<'USAGE'
Usage: scripts/package-hap.sh [--signed]

Build and package the arm64 app. Signing uses the local Hvigor signing profile.
NATIVE_PROFILE and HAP_BUILD_MODE accept release (default) or debug.
Set DEVECO_STUDIO_HOME to the DevEco Studio Contents directory if needed.
OUTPUT_HAP optionally copies the resulting HAP to another path.
USAGE
    exit 0
    ;;
esac
if [ "$#" -ne 0 ]; then
  echo "Unknown argument: $1 (use --help)" >&2
  exit 2
fi

DEVECO_STUDIO_HOME="${DEVECO_STUDIO_HOME:-/Applications/DevEco-Studio.app/Contents}"
OHRS="${OHRS:-ohrs}"
ARCH="${ARCH:-aarch}"
NATIVE_PROFILE="${NATIVE_PROFILE:-release}"
HAP_BUILD_MODE="${HAP_BUILD_MODE:-release}"
case "$ARCH" in
  aarch|arm64) ARCH=aarch ;;
  *)
    echo "The current HAP profile only includes arm64-v8a (ARCH=aarch or arm64)." >&2
    exit 2
    ;;
esac
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
if [ "$SIGNED" -eq 1 ]; then
  HAP_PATH="${SIGNED_HAP:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-signed.hap}"
  OUTPUT_HAP="${OUTPUT_HAP:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-$HAP_BUILD_MODE-signed.hap}"
fi
HVIGOR_ARGS="${HVIGOR_ARGS:---no-daemon}"

if [ -n "${HVIGORW:-}" ]; then
  HVIGORW_BIN="$(command -v "$HVIGORW" || true)"
elif [ -x "$ROOT_DIR/hvigorw" ]; then
  HVIGORW_BIN="$ROOT_DIR/hvigorw"
elif [ -x "$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw" ]; then
  HVIGORW_BIN="$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw"
else
  HVIGORW_BIN="$(command -v hvigorw || true)"
fi
if [ ! -x "$HVIGORW_BIN" ]; then
  echo "Hvigor was not found. Set HVIGORW or DEVECO_STUDIO_HOME." >&2
  exit 127
fi

if [ -n "${DEVECO_JAVA_HOME:-}" ]; then
  export JAVA_HOME="$DEVECO_JAVA_HOME"
elif [ -d "$DEVECO_STUDIO_HOME/jbr/Contents/Home" ]; then
  export JAVA_HOME="$DEVECO_STUDIO_HOME/jbr/Contents/Home"
fi
if [ -n "${JAVA_HOME:-}" ]; then
  export PATH="$JAVA_HOME/bin:$PATH"
fi
if [ "$SIGNED" -eq 1 ]; then
  if [ ! -x "${JAVA_HOME:-}/bin/java" ]; then
    echo "Signing requires DevEco Studio JBR. Set DEVECO_STUDIO_HOME or DEVECO_JAVA_HOME." >&2
    exit 1
  fi
  command -v zipinfo >/dev/null
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

if [ "$NATIVE_PROFILE" = "release" ]; then
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH" --package paws_ui --release -- --locked
else
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH" --package paws_ui -- --locked
fi
# Hvigor can hard-link its native intermediates to the input library. Avoid
# rewriting that inode; install a fresh file atomically instead. A buffered
# copy also avoids macOS copyfile's slow sparse seeks on large debug ELFs.
mkdir -p "$NATIVE_LIB_DIR"
NATIVE_STAGE=""
trap 'rm -f "$NATIVE_STAGE"' EXIT
trap 'exit 130' HUP INT TERM
install_library() {
  NATIVE_STAGE="$(mktemp "$NATIVE_LIB_DIR/.native.XXXXXX")"
  dd if="$1" of="$NATIVE_STAGE" bs=1048576
  chmod 755 "$NATIVE_STAGE"
  mv -f "$NATIVE_STAGE" "$2"
}
install_library "$SO_SRC" "$SO_DST"
install_library "$CXX_SHARED_SRC" "$CXX_SHARED_DST"

TASK=default@PackageHap
if [ "$SIGNED" -eq 1 ]; then
  TASK=assembleHap
fi
# Reject a stale artifact if Hvigor exits successfully without producing a HAP.
rm -f "$HAP_PATH"
"$HVIGORW_BIN" "$TASK" --mode module -p module=entry@default \
  -p buildMode="$HAP_BUILD_MODE" $HVIGOR_ARGS

if [ ! -f "$HAP_PATH" ]; then
  echo "Expected HAP was not generated: $HAP_PATH" >&2
  exit 1
fi

if [ "$SIGNED" -eq 1 ]; then
  ICON_LAYOUT="$(zipinfo -l "$HAP_PATH" | awk '/resources\.index|resources\/base\/media\/(background|foreground|layered_image)/')"
  ICON_ENTRY_COUNT="$(printf '%s\n' "$ICON_LAYOUT" | awk 'NF { count += 1 } END { print count + 0 }')"
  if [ "$ICON_ENTRY_COUNT" -ne 4 ]; then
    echo "Signed HAP is missing application icon resources." >&2
    exit 1
  fi
  if printf '%s\n' "$ICON_LAYOUT" | grep -Eq ' def[NX]?[[:space:]]'; then
    echo "Signed HAP compressed icon resources and is unsafe for physical-device launchers." >&2
    exit 1
  fi
fi
if [ -n "${OUTPUT_HAP:-}" ] && [ "$OUTPUT_HAP" != "$HAP_PATH" ]; then
  cp "$HAP_PATH" "$OUTPUT_HAP"
  HAP_PATH="$OUTPUT_HAP"
fi
echo "$HAP_PATH"

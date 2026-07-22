#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEVECO_STUDIO_HOME="${DEVECO_STUDIO_HOME:-/Applications/DevEco-Studio.app/Contents}"
HAP_BUILD_MODE="${HAP_BUILD_MODE:-release}"
SIGNED_HAP="${SIGNED_HAP:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-signed.hap}"
OUTPUT_HAP="${OUTPUT_HAP:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-release-signed.hap}"
HVIGOR_ARGS="${HVIGOR_ARGS:---no-daemon}"

case "$HAP_BUILD_MODE" in
  release|debug)
    ;;
  *)
    echo "Unsupported HAP_BUILD_MODE: $HAP_BUILD_MODE (expected release or debug)" >&2
    exit 1
    ;;
esac

if [ -n "${HVIGORW:-}" ]; then
  HVIGORW_BIN="$HVIGORW"
elif [ -x "$ROOT_DIR/hvigorw" ]; then
  HVIGORW_BIN="$ROOT_DIR/hvigorw"
elif [ -x "$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw" ]; then
  HVIGORW_BIN="$DEVECO_STUDIO_HOME/tools/hvigor/bin/hvigorw"
else
  HVIGORW_BIN="$(command -v hvigorw)"
fi

if [ -n "${DEVECO_JAVA_HOME:-}" ]; then
  export JAVA_HOME="$DEVECO_JAVA_HOME"
elif [ -d "$DEVECO_STUDIO_HOME/jbr/Contents/Home" ]; then
  export JAVA_HOME="$DEVECO_STUDIO_HOME/jbr/Contents/Home"
fi
if [ -n "${JAVA_HOME:-}" ]; then
  export PATH="$JAVA_HOME/bin:$PATH"
fi
if [ -z "${DEVECO_SDK_HOME:-}" ] && [ -d "$DEVECO_STUDIO_HOME/sdk" ]; then
  export DEVECO_SDK_HOME="$DEVECO_STUDIO_HOME/sdk"
fi
if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d "$DEVECO_STUDIO_HOME/sdk/default/openharmony" ]; then
  export OHOS_NDK_HOME="$DEVECO_STUDIO_HOME/sdk/default/openharmony"
fi

if [ ! -x "${JAVA_HOME:-}/bin/java" ]; then
  echo "DevEco Studio JBR is required to preserve and sign the Harmony HAP layout." >&2
  exit 1
fi

cd "$ROOT_DIR"
NATIVE_PROFILE="${NATIVE_PROFILE:-release}" HAP_BUILD_MODE="$HAP_BUILD_MODE" \
  "$ROOT_DIR/scripts/package-hap.sh"

"$HVIGORW_BIN" assembleHap --mode module -p module=entry@default \
  -p buildMode="$HAP_BUILD_MODE" $HVIGOR_ARGS

if [ ! -f "$SIGNED_HAP" ]; then
  echo "Expected signed HAP was not generated: $SIGNED_HAP" >&2
  exit 1
fi

ICON_LAYOUT="$(zipinfo -l "$SIGNED_HAP" | grep -E 'resources\.index|resources/base/media/(background|foreground|layered_image)')"
ICON_ENTRY_COUNT="$(printf '%s\n' "$ICON_LAYOUT" | awk 'NF { count += 1 } END { print count + 0 }')"
if [ "$ICON_ENTRY_COUNT" -ne 4 ]; then
  echo "Signed HAP is missing application icon resources." >&2
  exit 1
fi
if printf '%s\n' "$ICON_LAYOUT" | grep -Eq ' def[NX]?[[:space:]]'; then
  echo "Signed HAP compressed icon resources and is unsafe for physical-device launchers." >&2
  exit 1
fi

cp "$SIGNED_HAP" "$OUTPUT_HAP"
echo "$OUTPUT_HAP"

#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -z "${OHOS_NDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk/default/openharmony ]; then
  export OHOS_NDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony
fi
if [ -z "${DEVECO_SDK_HOME:-}" ] && [ -d /Applications/DevEco-Studio.app/Contents/sdk ]; then
  export DEVECO_SDK_HOME=/Applications/DevEco-Studio.app/Contents/sdk
fi
test -d "${OHOS_NDK_HOME:-}/native" || { echo "OpenHarmony NDK is unavailable" >&2; exit 1; }
cd "$ROOT_DIR"
OHRS_BIN=${OHRS:-ohrs}
OHRS_LOG=$(mktemp "${TMPDIR:-/tmp}/paws-ohrs-build.XXXXXX")
OHRS_PATHS=$(mktemp "${TMPDIR:-/tmp}/paws-ohrs-paths.XXXXXX")
trap 'rm -f "$OHRS_LOG" "$OHRS_PATHS"' EXIT HUP INT TERM

# Some ohrs versions canonicalize every native search path from build scripts.
# boring-sys can emit build/{lib,crypto,ssl} even when CMake places both
# archives directly in build/, so a fresh target directory can make ohrs
# panic after Cargo has successfully built BoringSSL. Repair only those known
# generated directories and retry the incremental build.
attempt=1
while [ "$attempt" -le 8 ]; do
  : >"$OHRS_LOG"
  if "$OHRS_BIN" build "$@" >"$OHRS_LOG" 2>&1; then
    cat "$OHRS_LOG"
    exit 0
  else
    status=$?
  fi
  cat "$OHRS_LOG" >&2

  sed -n 's/^Convert native=\(.*\) to absolute path failed\..*$/\1/p' \
    "$OHRS_LOG" >"$OHRS_PATHS"
  if [ ! -s "$OHRS_PATHS" ]; then
    exit "$status"
  fi

  repaired=0
  while IFS= read -r missing_path; do
    case "$missing_path" in
      "$ROOT_DIR"/target/*/build/boring-sys-*/out/build/lib | \
      "$ROOT_DIR"/target/*/build/boring-sys-*/out/build/crypto | \
      "$ROOT_DIR"/target/*/build/boring-sys-*/out/build/ssl)
        build_dir=${missing_path%/*}
        mkdir -p "$build_dir/lib" "$build_dir/crypto" "$build_dir/ssl"
        repaired=1
        ;;
      *)
        echo "Refusing to create unexpected ohrs path: $missing_path" >&2
        exit "$status"
        ;;
    esac
  done <"$OHRS_PATHS"

  if [ "$repaired" -ne 1 ]; then
    exit "$status"
  fi
  echo "Retrying ohrs after repairing generated boring-sys search directories." >&2
  attempt=$((attempt + 1))
done

echo "ohrs still failed after repairing generated boring-sys directories." >&2
exit 1

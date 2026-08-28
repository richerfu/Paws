#!/usr/bin/env sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OHRS_BIN=${OHRS:-ohrs}
OHRS_LOG=$(mktemp "${TMPDIR:-/tmp}/paws-ohrs-build.XXXXXX")
OHRS_PATHS=$(mktemp "${TMPDIR:-/tmp}/paws-ohrs-paths.XXXXXX")
trap 'rm -f "$OHRS_LOG" "$OHRS_PATHS"' EXIT HUP INT TERM

# ohrs 1.4.2 canonicalizes every native search path emitted by build scripts.
# boring-sys 5.1 emits build/{lib,crypto,ssl} even when CMake places both
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

#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SOURCE="${1:-${GO_OHOS_ROOT:-}}"
test -f "$SOURCE/VERSION" || { echo "OpenHarmony Go distribution not found: $SOURCE" >&2; exit 1; }
SOURCE="$(CDPATH= cd -- "$SOURCE" && pwd)"

usable_compiler() {
  [ -x "$1/bin/go" ] &&
    [ "$(GOTOOLCHAIN=local "$1/bin/go" env GOVERSION 2>/dev/null)" = "go1.24.7" ] &&
    "$1/bin/go" tool dist list | grep -x 'openharmony/arm64' >/dev/null
}
if usable_compiler "$SOURCE"; then
  printf '%s\n' "$SOURCE"
  exit 0
fi

test "$(head -1 "$SOURCE/VERSION")" = go1.24.5 || {
  echo "Expected the OHOS Go 1.24.5 source distribution or a native OHOS Go 1.24.7 compiler." >&2
  exit 1
}
HOST="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
COMPILER="${GO_OHOS_HOST_ROOT:-$ROOT_DIR/target/toolchains/go1.24.7-ohos-$HOST}"
PATCH="$ROOT_DIR/patches/go-ohos-1.24.7-compat.patch"
SIGNATURE="$SOURCE:$(shasum -a 256 "$PATCH" | cut -d ' ' -f 1)"
if [ -d "$COMPILER" ]; then
  test "$(cat "$COMPILER/.paws-source" 2>/dev/null || true)" = "$SIGNATURE" || {
    echo "Compiler cache belongs to a different source; set an empty GO_OHOS_HOST_ROOT: $COMPILER" >&2
    exit 1
  }
else
  mkdir -p "$(dirname "$COMPILER")"
  cp -R "$SOURCE" "$COMPILER"
  patch --dry-run -p1 -F 0 -d "$COMPILER" -i "$PATCH" >&2
  patch -p1 -F 0 -d "$COMPILER" -i "$PATCH" >&2
  printf '%s\n' "$SIGNATURE" > "$COMPILER/.paws-source"
fi
if ! usable_compiler "$COMPILER"; then
  if [ -z "${GOROOT_BOOTSTRAP:-}" ]; then
    BOOTSTRAP="$ROOT_DIR/target/toolchains/$(basename "$SOURCE")-$HOST"
    if [ -x "$BOOTSTRAP/bin/go" ] && "$BOOTSTRAP/bin/go" version >/dev/null 2>&1; then
      export GOROOT_BOOTSTRAP="$BOOTSTRAP"
    fi
  fi
  (cd "$COMPILER/src" && bash make.bash) >&2
fi
usable_compiler "$COMPILER" || { echo "OHOS Go compiler validation failed." >&2; exit 1; }
printf '%s\n' "$COMPILER"

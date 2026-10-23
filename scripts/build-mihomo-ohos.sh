#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT_DIR/scripts/ohos-env.sh"
WORKSPACE_PARENT="$(cd "$ROOT_DIR/.." && pwd)"
if [ -z "${CLASH_META_ROOT:-}" ]; then
  if [ -d "$WORKSPACE_PARENT/Clash.Meta" ]; then
    CLASH_META_ROOT="$WORKSPACE_PARENT/Clash.Meta"
  elif [ -d "$ROOT_DIR/target/engine-sources/Clash.Meta" ]; then
    CLASH_META_ROOT="$ROOT_DIR/target/engine-sources/Clash.Meta"
  else
    CLASH_META_ROOT="$WORKSPACE_PARENT/FlClash-ohos-upstream/core/Clash.Meta"
  fi
fi
GO_OHOS_ROOT="${GO_OHOS_ROOT:-$WORKSPACE_PARENT/go-ohos-1.24.7-compat}"
if [ ! -d "$GO_OHOS_ROOT" ] && [ -d "$ROOT_DIR/target/toolchains/go1.24.7-ohos-darwin-arm64" ]; then
  GO_OHOS_ROOT="$ROOT_DIR/target/toolchains/go1.24.7-ohos-darwin-arm64"
fi
OHOS_NDK_HOME="${OHOS_NDK_HOME:-/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony}"
GO_COMPILER_ROOT="$(sh "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$GO_OHOS_ROOT")"
GO_BIN="$GO_COMPILER_ROOT/bin/go"
CC_BIN="$OHOS_NDK_HOME/native/llvm/bin/clang"
CXX_BIN="$OHOS_NDK_HOME/native/llvm/bin/clang++"
SYSROOT="$OHOS_NDK_HOME/native/sysroot"
CLASH_OUTPUT="${MIHOMO_CORE_OUTPUT:-$ROOT_DIR/entry/libs/arm64-v8a/libclash.so}"
GVISOR_MOD="github.com/metacubex/gvisor@v0.0.0-20260922041103-e2cbcd6e7400"

test -x "$GO_BIN" || { echo "OpenHarmony Go compiler not found: $GO_BIN" >&2; exit 1; }
test -x "$CC_BIN" || { echo "OpenHarmony clang not found: $CC_BIN" >&2; exit 1; }
test -x "$CXX_BIN" || { echo "OpenHarmony clang++ not found: $CXX_BIN" >&2; exit 1; }
test -d "$CLASH_META_ROOT" || { echo "Clash.Meta checkout not found: $CLASH_META_ROOT" >&2; exit 1; }

test "$(git -C "$CLASH_META_ROOT" rev-parse HEAD)" = "8597778c7df410ebe88c57e59eaab32d203156df" || {
  echo "Clash.Meta source revision mismatch (expected 8597778c7df410ebe88c57e59eaab32d203156df)" >&2
  exit 1
}

# Build only the exact pinned tree plus declared patches. Never reuse marker-
# based patches in a mutable checkout: their contents can change while HEAD
# remains the same, silently selecting stale native code.
BUILD_STAGE="${MIHOMO_BUILD_STAGE:-$ROOT_DIR/target/mihomo-ohos-build}"
test "$BUILD_STAGE/core/Clash.Meta" != "$CLASH_META_ROOT" || { echo "Build stage cannot be the source checkout" >&2; exit 1; }
rm -rf "$BUILD_STAGE"
mkdir -p "$BUILD_STAGE/core/Clash.Meta" "$BUILD_STAGE/.dart_tool/ohos" "$(dirname "$CLASH_OUTPUT")"
python3 "$ROOT_DIR/scripts/mihomo-source-inventory.py" "$ROOT_DIR" > "$BUILD_STAGE/inputs-before.json"
git -C "$CLASH_META_ROOT" archive 8597778c7df410ebe88c57e59eaab32d203156df -o "$BUILD_STAGE/source.tar"
tar -xf "$BUILD_STAGE/source.tar" -C "$BUILD_STAGE/core/Clash.Meta"
rm "$BUILD_STAGE/source.tar"
patch --batch --dry-run -p1 -F 0 -d "$BUILD_STAGE/core/Clash.Meta" -i "$ROOT_DIR/patches/mihomo-openharmony.patch"
patch --batch -p1 -F 0 -d "$BUILD_STAGE/core/Clash.Meta" -i "$ROOT_DIR/patches/mihomo-openharmony.patch"
patch --batch --dry-run -p1 -F 0 -d "$BUILD_STAGE/core/Clash.Meta" -i "$ROOT_DIR/patches/mihomo-paws-management.patch"
patch --batch -p1 -F 0 -d "$BUILD_STAGE/core/Clash.Meta" -i "$ROOT_DIR/patches/mihomo-paws-management.patch"
cp -R "$ROOT_DIR/vendor/mihomo-bridge/." "$BUILD_STAGE/core/"
for observation_source in journal.go wire.go public.go; do
  cp "$ROOT_DIR/vendor/paws-engine-observation/$observation_source" "$BUILD_STAGE/core/paws_$observation_source"
done

if grep -Fq '_ = syscall.Close(fd)' "$BUILD_STAGE/core/tun/tun.go"; then
  patch --dry-run -p1 -F 0 -d "$BUILD_STAGE" -i "$ROOT_DIR/patches/mihomo-ohos-tun-fd-ownership.patch"
  patch -p1 -F 0 -d "$BUILD_STAGE" -i "$ROOT_DIR/patches/mihomo-ohos-tun-fd-ownership.patch"
fi

GVISOR_DIR="$(cd "$BUILD_STAGE/core" && env GOTOOLCHAIN=local "$GO_BIN" mod download -json "$GVISOR_MOD" | python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["Dir"]) if "Dir" in data else sys.exit(data.get("Error", "Go module download did not return a source directory"))')"
cp -R "$GVISOR_DIR" "$BUILD_STAGE/.dart_tool/ohos/gvisor"
chmod -R u+w "$BUILD_STAGE/.dart_tool/ohos/gvisor"
patch --dry-run -p1 -F 0 -d "$BUILD_STAGE/.dart_tool/ohos/gvisor" -i "$ROOT_DIR/patches/gvisor-fstat.patch"
patch -p1 -F 0 -d "$BUILD_STAGE/.dart_tool/ohos/gvisor" -i "$ROOT_DIR/patches/gvisor-fstat.patch"

cd "$BUILD_STAGE/core"
env GOTOOLCHAIN=local GOOS=openharmony GOARCH=arm64 CGO_ENABLED=1 \
  CC="$CC_BIN --target=aarch64-linux-ohos --sysroot=$SYSROOT" \
  CXX="$CXX_BIN --target=aarch64-linux-ohos --sysroot=$SYSROOT" \
  "$GO_BIN" build -mod=mod -tags=with_gvisor -buildmode=c-shared \
  -trimpath -ldflags='-s -w' -o "$BUILD_STAGE/libclash.so" .

python3 "$ROOT_DIR/scripts/mihomo-source-inventory.py" "$ROOT_DIR" > "$BUILD_STAGE/inputs-after.json"
cmp -s "$BUILD_STAGE/inputs-before.json" "$BUILD_STAGE/inputs-after.json" || {
  echo 'Native Mihomo inputs changed during compilation; refusing to publish mixed artifact' >&2
  exit 1
}
cp "$BUILD_STAGE/libclash.so" "$CLASH_OUTPUT.tmp"
mv "$CLASH_OUTPUT.tmp" "$CLASH_OUTPUT"

rm -f "${CLASH_OUTPUT%.so}.h"
file "$CLASH_OUTPUT"
shasum -a 256 "$CLASH_OUTPUT"

#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_PARENT="$(cd "$ROOT_DIR/.." && pwd)"
if [ -z "${CLASH_META_ROOT:-}" ]; then
  if [ -d "$WORKSPACE_PARENT/Clash.Meta" ]; then
    CLASH_META_ROOT="$WORKSPACE_PARENT/Clash.Meta"
  else
    CLASH_META_ROOT="$WORKSPACE_PARENT/FlClash-ohos-upstream/core/Clash.Meta"
  fi
fi
GO_OHOS_ROOT="${GO_OHOS_ROOT:-$WORKSPACE_PARENT/go-ohos-1.24.7-compat}"
OHOS_NDK_HOME="${OHOS_NDK_HOME:-/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony}"
GO_COMPILER_ROOT="$(sh "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$GO_OHOS_ROOT")"
GO_BIN="$GO_COMPILER_ROOT/bin/go"
CC_BIN="$OHOS_NDK_HOME/native/llvm/bin/clang"
CXX_BIN="$OHOS_NDK_HOME/native/llvm/bin/clang++"
SYSROOT="$OHOS_NDK_HOME/native/sysroot"
CLASH_OUTPUT="${MIHOMO_CORE_OUTPUT:-$ROOT_DIR/entry/libs/arm64-v8a/libclash.so}"
WRAPPER_OUTPUT="${MIHOMO_WRAPPER_OUTPUT:-$ROOT_DIR/entry/libs/arm64-v8a/libcore.so}"
GVISOR_MOD="github.com/metacubex/gvisor@v0.0.0-20260922041103-e2cbcd6e7400"

test -x "$GO_BIN" || { echo "OpenHarmony Go compiler not found: $GO_BIN" >&2; exit 1; }
test -x "$CC_BIN" || { echo "OpenHarmony clang not found: $CC_BIN" >&2; exit 1; }
test -x "$CXX_BIN" || { echo "OpenHarmony clang++ not found: $CXX_BIN" >&2; exit 1; }
test -d "$CLASH_META_ROOT" || { echo "Clash.Meta checkout not found: $CLASH_META_ROOT" >&2; exit 1; }

test "$(git -C "$CLASH_META_ROOT" rev-parse HEAD)" = "8597778c7df410ebe88c57e59eaab32d203156df" || {
  echo "Clash.Meta source revision mismatch (expected 8597778c7df410ebe88c57e59eaab32d203156df)" >&2
  exit 1
}

if [ ! -f "$CLASH_META_ROOT/dns/patch_openharmony.go" ]; then
  git -C "$CLASH_META_ROOT" apply --check "$ROOT_DIR/patches/mihomo-openharmony.patch"
  git -C "$CLASH_META_ROOT" apply "$ROOT_DIR/patches/mihomo-openharmony.patch"
fi

BUILD_STAGE="$ROOT_DIR/target/mihomo-ohos-build"
rm -rf "$BUILD_STAGE"
mkdir -p "$BUILD_STAGE/core" "$BUILD_STAGE/.dart_tool/ohos" "$(dirname "$CLASH_OUTPUT")" "$(dirname "$WRAPPER_OUTPUT")"
cp -R "$ROOT_DIR/vendor/mihomo-bridge/." "$BUILD_STAGE/core/"
ln -s "$CLASH_META_ROOT" "$BUILD_STAGE/core/Clash.Meta"

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
  -trimpath -ldflags='-s -w' -o "$CLASH_OUTPUT" .

HEADER_DIR="$BUILD_STAGE/includes"
mkdir -p "$HEADER_DIR"
cp "${CLASH_OUTPUT%.so}.h" "$HEADER_DIR/libclash.h"
cp "$BUILD_STAGE/core/bride.h" "$HEADER_DIR/bride.h"
rm -f "${CLASH_OUTPUT%.so}.h"

"$CXX_BIN" --target=aarch64-linux-ohos --sysroot="$SYSROOT" \
  -shared -fPIC -O3 -flto -g0 -fno-exceptions -fno-rtti \
  -Wl,--gc-sections,--strip-all \
  -DLIBCLASH \
  -I"$HEADER_DIR" \
  -L"$(dirname "$CLASH_OUTPUT")" \
  "$ROOT_DIR/vendor/mihomo-bridge/ohos-napi/napi_init.cpp" \
  -lace_napi.z -lclash \
  -o "$WRAPPER_OUTPUT"

file "$CLASH_OUTPUT" "$WRAPPER_OUTPUT"
shasum -a 256 "$CLASH_OUTPUT" "$WRAPPER_OUTPUT"

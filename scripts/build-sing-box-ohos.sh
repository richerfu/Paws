#!/usr/bin/env sh
set -eu

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
. "$ROOT_DIR/scripts/ohos-env.sh"
WORKSPACE_PARENT="$(cd "$ROOT_DIR/.." && pwd)"
SING_BOX_ROOT="${SING_BOX_ROOT:-$WORKSPACE_PARENT/sing-box-ohos}"
if [ ! -d "$SING_BOX_ROOT" ] && [ -d "$ROOT_DIR/target/engine-sources/sing-box-ohos" ]; then
  SING_BOX_ROOT="$ROOT_DIR/target/engine-sources/sing-box-ohos"
fi
GO_OHOS_ROOT="${GO_OHOS_ROOT:-$WORKSPACE_PARENT/go-ohos-1.24.7-compat}"
if [ ! -d "$GO_OHOS_ROOT" ] && [ -d "$ROOT_DIR/target/toolchains/go1.24.7-ohos-darwin-arm64" ]; then
  GO_OHOS_ROOT="$ROOT_DIR/target/toolchains/go1.24.7-ohos-darwin-arm64"
fi
OHOS_NDK_HOME="${OHOS_NDK_HOME:-/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony}"
GO_COMPILER_ROOT="$(sh "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$GO_OHOS_ROOT")"
GO_BIN="$GO_COMPILER_ROOT/bin/go"
CC_BIN="$OHOS_NDK_HOME/native/llvm/bin/aarch64-unknown-linux-ohos-clang"
OUTPUT="${SING_BOX_OUTPUT:-$ROOT_DIR/entry/libs/arm64-v8a/libsingbox.so}"
BUILD_STAGE="${SING_BOX_BUILD_STAGE:-$ROOT_DIR/target/sing-box-ohos-build}"
PIN=628cb31ffa79cffffd34c2f9cde6cae044e4fc12

test -x "$GO_BIN" || { echo "OpenHarmony Go compiler not found: $GO_BIN" >&2; exit 1; }
test -x "$CC_BIN" || { echo "OpenHarmony clang not found: $CC_BIN" >&2; exit 1; }
test "$(git -C "$SING_BOX_ROOT" rev-parse HEAD)" = "$PIN" || {
  echo "sing-box source revision mismatch (expected $PIN)" >&2
  exit 1
}
# The owned stage is never a user checkout; HEAD plus marker inspection cannot
# establish the identity of previously patched, mutable upstream files.
test "$BUILD_STAGE" != "$SING_BOX_ROOT" || { echo 'Build stage cannot be the source checkout' >&2; exit 1; }
rm -rf "$BUILD_STAGE"
mkdir -p "$BUILD_STAGE/source" "$(dirname "$OUTPUT")"
python3 "$ROOT_DIR/scripts/sing-box-source-inventory.py" "$ROOT_DIR" > "$BUILD_STAGE/inputs-before.json"
git -C "$SING_BOX_ROOT" archive "$PIN" -o "$BUILD_STAGE/source.tar"
tar -xf "$BUILD_STAGE/source.tar" -C "$BUILD_STAGE/source"
rm "$BUILD_STAGE/source.tar"
for declared_patch in sing-box-http-obfs-handshake.patch sing-box-http-obfs-first-flight.patch sing-box-benchmark-probe.patch sing-box-paws-management.patch; do
  patch --batch --dry-run -p1 -F 0 -d "$BUILD_STAGE/source" -i "$ROOT_DIR/patches/$declared_patch"
  patch --batch -p1 -F 0 -d "$BUILD_STAGE/source" -i "$ROOT_DIR/patches/$declared_patch"
done
mkdir -p "$BUILD_STAGE/source/cmd/libtinypaws"
cp -R "$ROOT_DIR/vendor/sing-box-bridge/." "$BUILD_STAGE/source/cmd/libtinypaws/"
for observation_source in journal.go wire.go public.go; do
  cp "$ROOT_DIR/vendor/paws-engine-observation/$observation_source" "$BUILD_STAGE/source/cmd/libtinypaws/paws_$observation_source"
done
cd "$BUILD_STAGE/source"
env GOTOOLCHAIN=local GOOS=openharmony GOARCH=arm64 CGO_ENABLED=1 CC="$CC_BIN" \
  "$GO_BIN" build -trimpath -buildmode=c-shared \
  -tags 'with_gvisor,with_quic,with_dhcp,with_utls,with_clash_api' \
  -ldflags '-s -w' -o "$BUILD_STAGE/libsingbox.so" ./cmd/libtinypaws
python3 "$ROOT_DIR/scripts/sing-box-source-inventory.py" "$ROOT_DIR" > "$BUILD_STAGE/inputs-after.json"
cmp -s "$BUILD_STAGE/inputs-before.json" "$BUILD_STAGE/inputs-after.json" || {
  echo 'Native sing-box inputs changed during compilation; refusing to publish mixed artifact' >&2
  exit 1
}
cp "$BUILD_STAGE/libsingbox.so" "$OUTPUT.tmp"
mv "$OUTPUT.tmp" "$OUTPUT"
file "$OUTPUT"
shasum -a 256 "$OUTPUT"

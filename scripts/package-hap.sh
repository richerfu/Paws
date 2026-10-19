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
# Compile-time diagnostic overrides must never enter a production HAP.
if [ "$HAP_BUILD_MODE" = "release" ] && { [ "${PAWS_RAW_ENGINE_DIAGNOSTIC:-}" = "1" ] || [ "${PAWS_SOCKET_WIRE_CAPTURE:-}" = "1" ] || [ -n "${PAWS_BENCH_PEER_IP:-}" ] || [ -n "${PAWS_BENCH_NODE_FINGERPRINT:-}" ]; }; then
  echo "Release HAP refuses diagnostic socket capture or endpoint overrides." >&2
  exit 1
fi
NATIVE_LIB_DIR="$ROOT_DIR/entry/libs/arm64-v8a"
SO_DST="$NATIVE_LIB_DIR/libpaws_ui.so"
CXX_SHARED_DST="$NATIVE_LIB_DIR/libc++_shared.so"
WORKSPACE_PARENT="$(cd "$ROOT_DIR/.." && pwd)"
MIHOMO_OHOS_ROOT="${MIHOMO_OHOS_ROOT:-$WORKSPACE_PARENT/FlClash-ohos-upstream}"
MIHOMO_WRAPPER_SRC="${MIHOMO_WRAPPER_SRC:-$MIHOMO_OHOS_ROOT/ohos/core/build/default/intermediates/libs/default/arm64-v8a/libcore.so}"
MIHOMO_CORE_SRC="${MIHOMO_CORE_SRC:-$MIHOMO_OHOS_ROOT/ohos/core/libs/arm64-v8a/libclash.so}"
MIHOMO_TUN_SRC="${MIHOMO_TUN_SRC:-$MIHOMO_OHOS_ROOT/core/tun/tun.go}"
MIHOMO_WRAPPER_DST="$NATIVE_LIB_DIR/libcore.so"
MIHOMO_CORE_DST="$NATIVE_LIB_DIR/libclash.so"
SING_BOX_CORE_SRC="${SING_BOX_CORE_SRC:-$NATIVE_LIB_DIR/libsingbox.so}"
SING_BOX_CORE_DST="$NATIVE_LIB_DIR/libsingbox.so"
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
if [ ! -f "$MIHOMO_WRAPPER_SRC" ] || [ ! -f "$MIHOMO_CORE_SRC" ]; then
  echo "TinyPaws mihomo engine libraries were not found under $MIHOMO_OHOS_ROOT" >&2
  echo "Build FlClash's ohos/core first or set MIHOMO_WRAPPER_SRC and MIHOMO_CORE_SRC." >&2
  exit 1
fi
if [ ! -f "$MIHOMO_TUN_SRC" ]; then
  echo "TinyPaws mihomo TUN source was not found: $MIHOMO_TUN_SRC" >&2
  exit 1
fi
if grep -Fq '_ = syscall.Close(fd)' "$MIHOMO_TUN_SRC"; then
  echo "Mihomo still closes the HarmonyOS-owned TUN fd: $MIHOMO_TUN_SRC" >&2
  echo "Apply patches/mihomo-ohos-tun-fd-ownership.patch and rebuild libclash.so." >&2
  exit 1
fi
if [ "$MIHOMO_TUN_SRC" -nt "$MIHOMO_CORE_SRC" ]; then
  echo "Mihomo TUN source is newer than libclash.so; rebuild the HarmonyOS core first." >&2
  exit 1
fi
sing_box_stale=0
if [ ! -f "$SING_BOX_CORE_SRC" ]; then sing_box_stale=1; fi
for bridge_source in "$ROOT_DIR"/patches/sing-box-*.patch "$ROOT_DIR"/vendor/sing-box-bridge/*.go "$ROOT_DIR"/vendor/sing-box-bridge/httpobfs/*.go "$ROOT_DIR"/vendor/sing-box-bridge/tlsretry/*.go; do
  if [ "$bridge_source" -nt "$SING_BOX_CORE_SRC" ]; then sing_box_stale=1; fi
done
if [ "$sing_box_stale" -eq 1 ]; then "$ROOT_DIR/scripts/build-sing-box-ohos.sh"; fi
if [ -n "${DEVECO_NODE_HOME:-}" ]; then
  export NODE_HOME="$DEVECO_NODE_HOME"
elif [ -x "$DEVECO_STUDIO_HOME/tools/node/bin/node" ]; then
  export NODE_HOME="$DEVECO_STUDIO_HOME/tools/node"
fi
if [ -n "${NODE_HOME:-}" ]; then
  export PATH="$NODE_HOME/bin:$PATH"
fi

cd "$ROOT_DIR"
PAWS_ENGINE_BUILD_ID="$(python3 - "$ROOT_DIR" "$SING_BOX_CORE_SRC" "$MIHOMO_CORE_SRC" "$MIHOMO_WRAPPER_SRC" <<'PY_BUILD'
import hashlib, sys, os
from pathlib import Path
root=Path(sys.argv[1]); digest=hashlib.sha256()
files=[Path(p) for p in sys.argv[2:]]+[root/'Cargo.lock', root/'Cargo.toml']
for key in ['PAWS_RAW_ENGINE_DIAGNOSTIC','PAWS_BENCH_PEER_IP','PAWS_BENCH_NODE_FINGERPRINT','PAWS_SOCKET_WIRE_CAPTURE']:
    digest.update(key.encode()+b'\0'+os.environ.get(key,'').encode()+b'\0')
for directory in ['vendor/meow-listener','vendor/meow-common','vendor/meow-proxy','vendor/meow-anytls','vendor/meow-tunnel','vendor/sing-box-bridge','patches','crates/paws_core/src','crates/paws_vpn/src']:
    files.extend(p for p in (root/directory).rglob('*') if p.is_file() and p.suffix in ['.rs','.go','.toml','.patch'])
for path in sorted(files):
    digest.update(path.name.encode()+b'\0'); digest.update(path.read_bytes())
print(digest.hexdigest())
PY_BUILD
)"
export PAWS_ENGINE_BUILD_ID
if [ "$NATIVE_PROFILE" = "release" ]; then
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH" --release
else
  OHRS="$OHRS" scripts/ohrs-build.sh --arch "$ARCH"
fi
cp "$SO_SRC" "$SO_DST"
test -s "$ROOT_DIR/crates/paws_ui/dist/index.d.ts"
python3 - "$ROOT_DIR/crates/paws_ui/dist/index.d.ts" "$ROOT_DIR/entry/src/main/cpp/types/libpaws_ui/Index.d.ts" <<'PY_TYPES'
import sys
from pathlib import Path
Path(sys.argv[2]).write_text(Path(sys.argv[1]).read_text().rstrip() + "\n")
PY_TYPES
cp "$CXX_SHARED_SRC" "$CXX_SHARED_DST"
cp "$MIHOMO_WRAPPER_SRC" "$MIHOMO_WRAPPER_DST"
cp "$MIHOMO_CORE_SRC" "$MIHOMO_CORE_DST"
if [ "$SING_BOX_CORE_SRC" != "$SING_BOX_CORE_DST" ]; then
  cp "$SING_BOX_CORE_SRC" "$SING_BOX_CORE_DST"
fi
python3 "$ROOT_DIR/scripts/write-build-manifest.py" --hap-mode "$HAP_BUILD_MODE" --native-profile "$NATIVE_PROFILE"
"$HVIGORW_BIN" default@PackageHap --mode module -p module=entry@default \
  -p buildMode="$HAP_BUILD_MODE" $HVIGOR_ARGS

if [ ! -f "$HAP_PATH" ]; then
  echo "Expected unsigned HAP was not generated: $HAP_PATH" >&2
  exit 1
fi

python3 "$ROOT_DIR/scripts/write-build-manifest.py" --hap-mode "$HAP_BUILD_MODE" --native-profile "$NATIVE_PROFILE" --hap-path "$HAP_PATH"
echo "$HAP_PATH"

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
MIHOMO_WRAPPER_DST="$NATIVE_LIB_DIR/libcore.so"
MIHOMO_CORE_DST="$NATIVE_LIB_DIR/libclash.so"
if [ -d "$MIHOMO_OHOS_ROOT" ]; then
  MIHOMO_WRAPPER_SRC="${MIHOMO_WRAPPER_SRC:-$MIHOMO_OHOS_ROOT/ohos/core/build/default/intermediates/libs/default/arm64-v8a/libcore.so}"
  MIHOMO_CORE_SRC="${MIHOMO_CORE_SRC:-$MIHOMO_OHOS_ROOT/ohos/core/libs/arm64-v8a/libclash.so}"
  MIHOMO_TUN_SRC="${MIHOMO_TUN_SRC:-$MIHOMO_OHOS_ROOT/core/tun/tun.go}"
else
  MIHOMO_WRAPPER_SRC="${MIHOMO_WRAPPER_SRC:-$MIHOMO_WRAPPER_DST}"
  MIHOMO_CORE_SRC="${MIHOMO_CORE_SRC:-$MIHOMO_CORE_DST}"
  MIHOMO_TUN_SRC="${MIHOMO_TUN_SRC:-$ROOT_DIR/vendor/mihomo-bridge/tun/tun.go}"
fi
SING_BOX_CORE_SRC="${SING_BOX_CORE_SRC:-$NATIVE_LIB_DIR/libsingbox.so}"
SING_BOX_CORE_DST="$NATIVE_LIB_DIR/libsingbox.so"
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
mihomo_stale=0
if [ ! -f "$MIHOMO_WRAPPER_SRC" ] || [ ! -f "$MIHOMO_CORE_SRC" ]; then
  mihomo_stale=1
elif [ "$MIHOMO_WRAPPER_SRC" = "$MIHOMO_WRAPPER_DST" ] && [ "$MIHOMO_CORE_SRC" = "$MIHOMO_CORE_DST" ]; then
  # Rebuild repository-owned outputs after any bridge/toolchain patch changes.
  for mihomo_input in "$ROOT_DIR/scripts/build-mihomo-ohos.sh" "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$ROOT_DIR/patches/mihomo-openharmony.patch" "$ROOT_DIR/patches/mihomo-ohos-tun-fd-ownership.patch" "$ROOT_DIR/patches/gvisor-fstat.patch" "$ROOT_DIR/patches/go-ohos-1.24.7-compat.patch"; do
    if [ "$mihomo_input" -nt "$MIHOMO_CORE_SRC" ] || [ "$mihomo_input" -nt "$MIHOMO_WRAPPER_SRC" ]; then mihomo_stale=1; fi
  done
  if [ -n "$(find "$ROOT_DIR/vendor/mihomo-bridge" -type f \( -newer "$MIHOMO_CORE_SRC" -o -newer "$MIHOMO_WRAPPER_SRC" \) -print -quit)" ]; then mihomo_stale=1; fi
fi
if [ "$mihomo_stale" -eq 1 ]; then
  "$ROOT_DIR/scripts/build-mihomo-ohos.sh"
fi
if [ ! -f "$MIHOMO_WRAPPER_SRC" ] || [ ! -f "$MIHOMO_CORE_SRC" ]; then
  echo "TinyPaws mihomo engine libraries were not found ($MIHOMO_CORE_SRC, $MIHOMO_WRAPPER_SRC)" >&2
  echo "Run scripts/build-mihomo-ohos.sh or set MIHOMO_WRAPPER_SRC and MIHOMO_CORE_SRC." >&2
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
for bridge_source in "$ROOT_DIR/scripts/build-sing-box-ohos.sh" "$ROOT_DIR/scripts/prepare-go-ohos.sh" "$ROOT_DIR/patches/go-ohos-1.24.7-compat.patch" "$ROOT_DIR"/patches/sing-box-*.patch "$ROOT_DIR"/vendor/sing-box-bridge/*.go "$ROOT_DIR"/vendor/sing-box-bridge/httpobfs/*.go "$ROOT_DIR"/vendor/sing-box-bridge/tlsretry/*.go; do
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
for directory in ['vendor/meow-listener','vendor/meow-common','vendor/meow-proxy','vendor/meow-anytls','vendor/meow-tunnel','vendor/meow-transport','vendor/mihomo-bridge','vendor/sing-box-bridge','patches','crates/paws_core/src','crates/paws_vpn/src']:
    files.extend(p for p in (root/directory).rglob('*') if p.is_file() and p.suffix in ['.rs','.go','.c','.cpp','.h','.toml','.patch'])
for path in sorted(files):
    digest.update(path.name.encode()+b'\0'); digest.update(path.read_bytes())
print(digest.hexdigest())
PY_BUILD
)"
export PAWS_ENGINE_BUILD_ID
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
test -s "$ROOT_DIR/crates/paws_ui/dist/index.d.ts"
python3 - "$ROOT_DIR/crates/paws_ui/dist/index.d.ts" "$ROOT_DIR/entry/src/main/cpp/types/libpaws_ui/Index.d.ts" <<'PY_TYPES'
import sys
from pathlib import Path
Path(sys.argv[2]).write_text(Path(sys.argv[1]).read_text().rstrip() + "\n")
PY_TYPES
if [ "$MIHOMO_WRAPPER_SRC" != "$MIHOMO_WRAPPER_DST" ]; then
  install_library "$MIHOMO_WRAPPER_SRC" "$MIHOMO_WRAPPER_DST"
fi
if [ "$MIHOMO_CORE_SRC" != "$MIHOMO_CORE_DST" ]; then
  install_library "$MIHOMO_CORE_SRC" "$MIHOMO_CORE_DST"
fi
if [ "$SING_BOX_CORE_SRC" != "$SING_BOX_CORE_DST" ]; then
  install_library "$SING_BOX_CORE_SRC" "$SING_BOX_CORE_DST"
fi
python3 "$ROOT_DIR/scripts/write-build-manifest.py" --hap-mode "$HAP_BUILD_MODE" --native-profile "$NATIVE_PROFILE"

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
python3 "$ROOT_DIR/scripts/write-build-manifest.py" --hap-mode "$HAP_BUILD_MODE" --native-profile "$NATIVE_PROFILE" --hap-path "$HAP_PATH"
echo "$HAP_PATH"

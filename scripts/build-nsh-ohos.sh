#!/usr/bin/env sh
set -eu
ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$ROOT_DIR/scripts/ohos-env.sh"
NSH_ROOT=${NSH_ROOT:-$ROOT_DIR/../nsh}
NSH_ROOT=$(CDPATH= cd -- "$NSH_ROOT" && pwd)
export NSH_ROOT
# Stage the local SDK without modifying the checkout or Cargo's dependency
# cache. The current UIContext host uses NSH's own native editor and dialogs.
NSH_ROOT=$(python3 "$ROOT_DIR/scripts/prepare-nsh-build-source.py")
export NSH_ROOT
NSH_PROFILE=${NSH_PROFILE:-${NATIVE_PROFILE:-release}}
case "$NSH_PROFILE" in debug|release) ;; *) echo 'NSH_PROFILE must be debug or release' >&2; exit 2 ;; esac
NSH_OUTPUT="${NSH_SDK_OUTPUT:-$ROOT_DIR/dist/nsh-sdk/$NSH_PROFILE}"
NSH_TARGET="${NSH_TARGET_DIR:-$ROOT_DIR/target/nsh}"
case "$NSH_TARGET" in /*) ;; *) NSH_TARGET="$ROOT_DIR/$NSH_TARGET" ;; esac
export CARGO_TARGET_DIR="$NSH_TARGET"
NSH_CLI_SOURCE=local-checkout-build
if [ -n "${NSH_BIN:-}" ]; then NSH_CLI_SOURCE=explicit-override; fi
mkdir -p "$NSH_OUTPUT"
NSH_INPUTS=$(mktemp "$NSH_OUTPUT/source-inputs.XXXXXX")
trap 'rm -f "$NSH_INPUTS"' EXIT HUP INT TERM
python3 "$ROOT_DIR/scripts/nsh_build_inputs.py" > "$NSH_INPUTS"
# All generated artifacts stay inside Paws. The sibling NSH checkout is read-only.
if [ "${NSH_REUSE_SDK:-0}" = 1 ]; then
  python3 - "$ROOT_DIR/scripts" "$NSH_OUTPUT/source-inputs.json" "$NSH_INPUTS" "$NSH_OUTPUT/nsh" "${NSH_BIN:-}" <<'PY_REUSE'
import hashlib, json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from nsh_build_inputs import verify_source_identity
recorded, current = [json.loads(Path(path).read_text()) for path in sys.argv[2:4]]
try:
    verify_source_identity(recorded, current)
except ValueError as error:
    raise SystemExit(str(error)) from None
if not sys.argv[5]:
    with Path(sys.argv[4]).open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    if digest != recorded.get('cli', {}).get('sha256'):
        raise SystemExit('The saved NSH CLI differs from the build record; rebuild before reuse')
PY_REUSE
  NSH_BIN=${NSH_BIN:-$NSH_OUTPUT/nsh}
  test -x "$NSH_BIN" || { echo 'NSH_REUSE_SDK requires the previously built CLI' >&2; exit 1; }
else
  cd "$NSH_ROOT"
  # A failed ohrs invocation can return status zero; reject stale output as well.
  rm -f "$NSH_OUTPUT/arm64-v8a/libnsh_platform_ohos.so" "$NSH_OUTPUT/index.d.ts"
  if [ "$NSH_PROFILE" = release ]; then
    "${OHRS:-ohrs}" build -p nsh-platform-ohos -a aarch --release --dist "$NSH_OUTPUT" --target-dir "$NSH_TARGET" -- --locked
  else
    "${OHRS:-ohrs}" build -p nsh-platform-ohos -a aarch --dist "$NSH_OUTPUT" --target-dir "$NSH_TARGET" -- --locked
  fi
fi
test -s "$NSH_OUTPUT/arm64-v8a/libnsh_platform_ohos.so" || { echo 'NSH native build produced no library' >&2; exit 1; }
test -s "$NSH_OUTPUT/index.d.ts" || { echo 'NSH native build produced no declarations' >&2; exit 1; }
if [ -z "${NSH_BIN:-}" ]; then
  cargo build --manifest-path "$NSH_ROOT/Cargo.toml" --target-dir "$NSH_TARGET" -p nsh-cli --locked
  cp "$NSH_TARGET/debug/nsh" "$NSH_OUTPUT/nsh"
  NSH_BIN="$NSH_OUTPUT/nsh"
fi
python3 - "$ROOT_DIR/scripts" "$NSH_INPUTS" "$NSH_OUTPUT/source-inputs.json" "$NSH_BIN" "$NSH_CLI_SOURCE" <<'PY_ATTEST'
import hashlib, json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from nsh_build_inputs import resolve_inputs, verify_source_identity
inputs = json.loads(Path(sys.argv[2]).read_text())
verify_source_identity(inputs, resolve_inputs())
cli = Path(sys.argv[4]).resolve(strict=True)
digest = hashlib.sha256()
with cli.open('rb') as stream:
    for chunk in iter(lambda: stream.read(1 << 20), b''):
        digest.update(chunk)
inputs['cli'] = {'path': str(cli), 'sha256': digest.hexdigest(), 'sourceKind': sys.argv[5]}
Path(sys.argv[3]).write_text(json.dumps(inputs, indent=2) + '\n')
PY_ATTEST
cd "$ROOT_DIR"
mkdir -p target/ohos/paws-bundle entry/src/main/resources/rawfile entry/src/main/cpp/types/libnsh_platform_ohos entry/libs/arm64-v8a
"$NSH_BIN" build "$ROOT_DIR/ui/nsh" -o "$ROOT_DIR/target/ohos/paws-bundle"
cp "$ROOT_DIR/target/ohos/paws-bundle/bundle.js" "$ROOT_DIR/entry/src/main/resources/rawfile/paws.bundle.js"
if [ -d "$ROOT_DIR/target/ohos/paws-bundle/assets" ]; then
  mkdir -p "$ROOT_DIR/entry/src/main/resources/rawfile/assets"
  cp -R "$ROOT_DIR/target/ohos/paws-bundle/assets/." "$ROOT_DIR/entry/src/main/resources/rawfile/assets/"
fi
cp "$NSH_OUTPUT/arm64-v8a/libnsh_platform_ohos.so" "$ROOT_DIR/entry/libs/arm64-v8a/libnsh_platform_ohos.so"
cp "$NSH_OUTPUT/index.d.ts" "$ROOT_DIR/entry/src/main/cpp/types/libnsh_platform_ohos/Index.d.ts"
cp "$NSH_OUTPUT/source-inputs.json" "$ROOT_DIR/target/ohos/nsh-source-inputs.json"
python3 "$ROOT_DIR/scripts/prepare-nsh-ability.py"
ohpm install --all

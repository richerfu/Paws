#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
HDC="${HDC:-hdc}"
HDC_TARGET="${HDC_TARGET:-}"
BUNDLE_NAME="${BUNDLE_NAME:-com.richerfu.paws}"
ABILITY_NAME="${ABILITY_NAME:-EntryAbility}"
HAP_PATH="${HAP_PATH:-}"
PROFILE_URL="${PROFILE_URL:-http://10.0.2.2:8766/direct.yaml}"
PROFILE_NAME="${PROFILE_NAME:-Meow订阅交互测试}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/smoke-logs}"
RESET_APP_DATA="${RESET_APP_DATA:-1}"

usage() {
  cat <<USAGE
Usage: scripts/harmony-subscription-ui-smoke.sh --hap DEBUG_HAP

Installs a debug HAP, imports a subscription through debug-only Want
automation, and verifies the subscription UI. Release HAPs are rejected
because EntryAbility intentionally ignores automation Want parameters.

Options:
  --hap PATH  Install and test this debug HAP.
  -h, --help  Show this help.

Environment overrides:
  HDC, HDC_TARGET, BUNDLE_NAME, ABILITY_NAME, HAP_PATH, PROFILE_URL,
  PROFILE_NAME, LOG_DIR, RESET_APP_DATA
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --hap)
      HAP_PATH="${2:?missing HAP path}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

hdc_cmd() {
  if [ -n "$HDC_TARGET" ]; then
    "$HDC" -t "$HDC_TARGET" "$@"
  else
    "$HDC" "$@"
  fi
}

require_debug_automation_hap() {
  if [ -z "$HAP_PATH" ]; then
    echo "Subscription Want automation requires --hap DEBUG_HAP (or HAP_PATH)." >&2
    echo "Release EntryAbility ignores these Want parameters, so the script fails closed." >&2
    exit 2
  fi
  if [ ! -f "$HAP_PATH" ]; then
    echo "HAP not found: $HAP_PATH" >&2
    exit 1
  fi
  if ! unzip -p "$HAP_PATH" module.json 2>/dev/null \
    | jq -e '.app.debug == true' >/dev/null 2>&1; then
    echo "Subscription Want automation requires a debug HAP (module.json app.debug=true)." >&2
    echo "The selected HAP is release or its manifest could not be read: $HAP_PATH" >&2
    exit 1
  fi
}

dump_layout() {
  remote_path="$1"
  local_path="$2"
  hdc_cmd shell uitest dumpLayout -p "$remote_path" -a -b "$BUNDLE_NAME" >/dev/null
  hdc_cmd file recv "$remote_path" "$local_path" >/dev/null
}

capture_screen() {
  remote_path="$1"
  local_path="$2"
  hdc_cmd shell snapshot_display -f "$remote_path" >/dev/null
  hdc_cmd file recv "$remote_path" "$local_path" >/dev/null
}

assert_text() {
  layout="$1"
  expected="$2"
  if ! jq -e --arg expected "$expected" \
    '.. | objects | select(.attributes?.text == $expected)' "$layout" >/dev/null; then
    printf 'Missing UI text: %s\n' "$expected" >&2
    exit 1
  fi
}

click_text() {
  layout="$1"
  expected="$2"
  occurrence="${3:-last}"
  bounds="$(jq -r --arg expected "$expected" --arg occurrence "$occurrence" '
    [.. | objects | select(.attributes?.text == $expected) | .attributes.bounds]
    | if $occurrence == "first" then first else last end // empty
  ' "$layout")"
  if [ -z "$bounds" ]; then
    printf 'Cannot click missing UI text: %s\n' "$expected" >&2
    exit 1
  fi
  coordinates="$(printf '%s' "$bounds" | sed -e 's/\[/ /g' -e 's/\]/ /g' -e 's/,/ /g')"
  set -- $coordinates
  x="$((($1 + $3) / 2))"
  y="$((($2 + $4) / 2))"
  hdc_cmd shell uitest uiInput click "$x" "$y" >/dev/null
}

mkdir -p "$LOG_DIR"
command -v jq >/dev/null
command -v unzip >/dev/null
require_debug_automation_hap

# Install the same HAP whose manifest was checked. Merely inspecting a local
# debug artifact would not prove that the device is running a debug package.
hdc_cmd install -r "$HAP_PATH" >/dev/null

if [ "$RESET_APP_DATA" = "1" ]; then
  hdc_cmd shell aa force-stop "$BUNDLE_NAME" >/dev/null 2>&1 || true
  hdc_cmd shell bm clean -n "$BUNDLE_NAME" -d >/dev/null
fi

hdc_cmd shell aa start \
  -b "$BUNDLE_NAME" \
  -a "$ABILITY_NAME" \
  --ps pawsProfileUrl "$PROFILE_URL" \
  --ps pawsProfileName "$PROFILE_NAME" >/dev/null
# Cold startup includes native library initialization and subscription download.
# The emulator regularly needs around three seconds before the first snapshot.
sleep 6

home_layout="$LOG_DIR/paws-subscription-home.json"
list_layout="$LOG_DIR/paws-subscription-list.json"
menu_layout="$LOG_DIR/paws-subscription-actions.json"
edit_layout="$LOG_DIR/paws-subscription-edit.json"

dump_layout /data/local/tmp/paws-subscription-home.json "$home_layout"
assert_text "$home_layout" "$PROFILE_NAME"
assert_text "$home_layout" "首页"

click_text "$home_layout" "订阅" last
sleep 1
dump_layout /data/local/tmp/paws-subscription-list.json "$list_layout"
assert_text "$list_layout" "订阅"
assert_text "$list_layout" "$PROFILE_NAME"
assert_text "$list_layout" "$PROFILE_URL"
if ! jq -e '.. | objects | select(.attributes?.text | type == "string" and endswith(" UTC"))' \
  "$list_layout" >/dev/null; then
  printf 'Imported subscription card has no updated timestamp\n' >&2
  exit 1
fi
capture_screen /data/local/tmp/paws-subscription-list.jpeg \
  "$LOG_DIR/paws-subscription-list.jpeg"

name_bounds="$(jq -r --arg expected "$PROFILE_NAME" \
  '[.. | objects | select(.attributes?.text == $expected) | .attributes.bounds] | last // empty' \
  "$list_layout")"
coordinates="$(printf '%s' "$name_bounds" | sed -e 's/\[/ /g' -e 's/\]/ /g' -e 's/,/ /g')"
set -- $coordinates
menu_y="$(($4 + 50))"
display_right="$(jq -r '.children[0].attributes.bounds' "$list_layout" | \
  sed -e 's/.*\]\[//' -e 's/,.*//')"
menu_x="$((display_right - 90))"
hdc_cmd shell uitest uiInput click "$menu_x" "$menu_y" >/dev/null
sleep 1

dump_layout /data/local/tmp/paws-subscription-actions.json "$menu_layout"
for action in "编辑订阅" "编辑 YAML" "导出配置" "刷新订阅" "删除配置"; do
  assert_text "$menu_layout" "$action"
done
capture_screen /data/local/tmp/paws-subscription-actions.jpeg \
  "$LOG_DIR/paws-subscription-actions.jpeg"

click_text "$menu_layout" "编辑订阅" last
sleep 1
dump_layout /data/local/tmp/paws-subscription-edit.json "$edit_layout"
assert_text "$edit_layout" "编辑订阅"
assert_text "$edit_layout" "名称"
assert_text "$edit_layout" "订阅地址"
assert_text "$edit_layout" "保存修改"
capture_screen /data/local/tmp/paws-subscription-edit.jpeg \
  "$LOG_DIR/paws-subscription-edit.jpeg"

printf 'Subscription UI smoke passed. Evidence: %s\n' "$LOG_DIR"

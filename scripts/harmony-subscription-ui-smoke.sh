#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
HDC="${HDC:-hdc}"
HDC_TARGET="${HDC_TARGET:-}"
BUNDLE_NAME="${BUNDLE_NAME:-com.richerfu.clash_hmeta}"
ABILITY_NAME="${ABILITY_NAME:-EntryAbility}"
PROFILE_URL="${PROFILE_URL:-http://10.0.2.2:8766/direct.yaml}"
PROFILE_NAME="${PROFILE_NAME:-Meow订阅交互测试}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/smoke-logs}"
RESET_APP_DATA="${RESET_APP_DATA:-1}"

hdc_cmd() {
  if [ -n "$HDC_TARGET" ]; then
    "$HDC" -t "$HDC_TARGET" "$@"
  else
    "$HDC" "$@"
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

if [ "$RESET_APP_DATA" = "1" ]; then
  hdc_cmd shell aa force-stop "$BUNDLE_NAME" >/dev/null 2>&1 || true
  hdc_cmd shell bm clean -n "$BUNDLE_NAME" -d >/dev/null
fi

hdc_cmd shell aa start \
  -b "$BUNDLE_NAME" \
  -a "$ABILITY_NAME" \
  --ps hmetaProfileUrl "$PROFILE_URL" \
  --ps hmetaProfileName "$PROFILE_NAME" >/dev/null
# Cold startup includes native library initialization and subscription download.
# The emulator regularly needs around three seconds before the first snapshot.
sleep 6

home_layout="$LOG_DIR/hmeta-subscription-home.json"
list_layout="$LOG_DIR/hmeta-subscription-list.json"
menu_layout="$LOG_DIR/hmeta-subscription-actions.json"
edit_layout="$LOG_DIR/hmeta-subscription-edit.json"

dump_layout /data/local/tmp/hmeta-subscription-home.json "$home_layout"
assert_text "$home_layout" "$PROFILE_NAME"
assert_text "$home_layout" "首页"

click_text "$home_layout" "订阅" last
sleep 1
dump_layout /data/local/tmp/hmeta-subscription-list.json "$list_layout"
assert_text "$list_layout" "订阅"
assert_text "$list_layout" "$PROFILE_NAME"
assert_text "$list_layout" "$PROFILE_URL"
if ! jq -e '.. | objects | select(.attributes?.text | type == "string" and endswith(" UTC"))' \
  "$list_layout" >/dev/null; then
  printf 'Imported subscription card has no updated timestamp\n' >&2
  exit 1
fi
capture_screen /data/local/tmp/hmeta-subscription-list.jpeg \
  "$LOG_DIR/hmeta-subscription-list.jpeg"

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

dump_layout /data/local/tmp/hmeta-subscription-actions.json "$menu_layout"
for action in "编辑订阅" "编辑 YAML" "导出配置" "刷新订阅" "删除配置"; do
  assert_text "$menu_layout" "$action"
done
capture_screen /data/local/tmp/hmeta-subscription-actions.jpeg \
  "$LOG_DIR/hmeta-subscription-actions.jpeg"

click_text "$menu_layout" "编辑订阅" last
sleep 1
dump_layout /data/local/tmp/hmeta-subscription-edit.json "$edit_layout"
assert_text "$edit_layout" "编辑订阅"
assert_text "$edit_layout" "名称"
assert_text "$edit_layout" "订阅地址"
assert_text "$edit_layout" "保存修改"
capture_screen /data/local/tmp/hmeta-subscription-edit.jpeg \
  "$LOG_DIR/hmeta-subscription-edit.jpeg"

printf 'Subscription UI smoke passed. Evidence: %s\n' "$LOG_DIR"

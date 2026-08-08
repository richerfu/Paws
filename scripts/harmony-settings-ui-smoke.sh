#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
HDC="${HDC:-hdc}"
HDC_TARGET="${HDC_TARGET:-}"
BUNDLE_NAME="${BUNDLE_NAME:-com.richerfu.paws}"
ABILITY_NAME="${ABILITY_NAME:-EntryAbility}"
HAP_PATH="${HAP_PATH:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-unsigned.hap}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/smoke-logs}"
INSTALL_HAP="${INSTALL_HAP:-1}"

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
  jq -e --arg expected "$expected" \
    '.. | objects | select(.attributes?.text == $expected)' "$layout" >/dev/null || {
    printf 'Missing UI text: %s\n' "$expected" >&2
    exit 1
  }
}

assert_no_text() {
  layout="$1"
  unexpected="$2"
  if jq -e --arg unexpected "$unexpected" \
    '.. | objects | select(.attributes?.text == $unexpected)' "$layout" >/dev/null; then
    printf 'Unexpected UI text: %s\n' "$unexpected" >&2
    exit 1
  fi
}

text_bounds() {
  layout="$1"
  expected="$2"
  occurrence="${3:-last}"
  jq -r --arg expected "$expected" --arg occurrence "$occurrence" '
    [.. | objects | select(.attributes?.type == "Text" and .attributes?.text == $expected) | .attributes.bounds]
    | if $occurrence == "first" then first else last end // empty
  ' "$layout"
}

click_text() {
  bounds="$(text_bounds "$1" "$2" "${3:-last}")"
  if [ -z "$bounds" ]; then
    printf 'Cannot click missing UI text: %s\n' "$2" >&2
    exit 1
  fi
  coordinates="$(printf '%s' "$bounds" | sed -e 's/\[/ /g' -e 's/\]/ /g' -e 's/,/ /g')"
  set -- $coordinates
  hdc_cmd shell uitest uiInput click "$((($1 + $3) / 2))" "$((($2 + $4) / 2))" >/dev/null
}

left_edge() {
  text_bounds "$1" "$2" first | sed -e 's/^\[//' -e 's/,.*//'
}

top_edge() {
  text_bounds "$1" "$2" last | sed -e 's/^\[[^,]*,//' -e 's/\].*//'
}

mkdir -p "$LOG_DIR"
command -v jq >/dev/null

if [ "$INSTALL_HAP" = "1" ]; then
  hdc_cmd install -r "$HAP_PATH" >/dev/null
fi
hdc_cmd shell aa force-stop "$BUNDLE_NAME" >/dev/null 2>&1 || true
hdc_cmd shell aa start -b "$BUNDLE_NAME" -a "$ABILITY_NAME" >/dev/null
sleep 5

home_layout="$LOG_DIR/paws-settings-home.json"
root_layout="$LOG_DIR/paws-settings-alignment.json"
network_layout="$LOG_DIR/paws-network-child.json"
about_layout="$LOG_DIR/paws-about-optimized.json"

dump_layout /data/local/tmp/paws-settings-home.json "$home_layout"
click_text "$home_layout" "设置" last
sleep 1
dump_layout /data/local/tmp/paws-settings-alignment.json "$root_layout"
for text in "常规" "版本" "引擎" "网络设置"; do
  assert_text "$root_layout" "$text"
done
assert_no_text "$root_layout" "分应用 VPN"

version_left="$(left_edge "$root_layout" "版本")"
engine_left="$(left_edge "$root_layout" "引擎")"
if [ "$version_left" != "$engine_left" ]; then
  printf 'Settings labels are not aligned: version=%s engine=%s\n' \
    "$version_left" "$engine_left" >&2
  exit 1
fi
capture_screen /data/local/tmp/paws-settings-alignment.jpeg \
  "$LOG_DIR/paws-settings-alignment.jpeg"

click_text "$root_layout" "网络设置" last
sleep 1
dump_layout /data/local/tmp/paws-network-child.json "$network_layout"
assert_text "$network_layout" "网络设置"
bottom_tab_count="$(jq '[.. | objects | select(.attributes?.text == "首页" or .attributes?.text == "订阅" or .attributes?.text == "流量")] | length' "$network_layout")"
if [ "$bottom_tab_count" -ne 0 ]; then
  printf 'Secondary network page still renders the main bottom navigation\n' >&2
  exit 1
fi
capture_screen /data/local/tmp/paws-network-child.jpeg \
  "$LOG_DIR/paws-network-child.jpeg"

# The secondary-page back action is the top-left 40 vp icon button.
hdc_cmd shell uitest uiInput click 80 220 >/dev/null
sleep 1
hdc_cmd shell uitest uiInput swipe 660 2200 660 700 600 >/dev/null
sleep 1
dump_layout /data/local/tmp/paws-settings-about-entry.json \
  "$LOG_DIR/paws-settings-about-entry.json"
click_text "$LOG_DIR/paws-settings-about-entry.json" "关于" last
sleep 1
dump_layout /data/local/tmp/paws-about-optimized.json "$about_layout"
for text in "Paws" "隐私" "meow-rs" "arkit"; do
  assert_text "$about_layout" "$text"
done

revision="$(jq -r '[.. | objects | .attributes?.text? | select(type == "string" and test("^[0-9a-f]{6,}…[0-9a-f]{4,}$"))] | first // empty' "$about_layout")"
revision_length="$(jq -nr --arg revision "$revision" '$revision | length')"
if [ -z "$revision" ] || [ "$revision_length" -gt 18 ]; then
  printf 'Arkit revision is not safely middle-truncated: %s\n' "$revision" >&2
  exit 1
fi

meow_top="$(top_edge "$about_layout" "meow-rs")"
arkit_top="$(top_edge "$about_layout" "arkit")"
if [ "$meow_top" != "$arkit_top" ]; then
  printf 'Repository labels are not on the same baseline: meow-rs=%s arkit=%s\n' \
    "$meow_top" "$arkit_top" >&2
  exit 1
fi
capture_screen /data/local/tmp/paws-about-optimized.jpeg \
  "$LOG_DIR/paws-about-optimized.jpeg"

printf 'Settings/About UI smoke passed. Evidence: %s\n' "$LOG_DIR"

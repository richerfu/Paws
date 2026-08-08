#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
HDC="${HDC:-hdc}"
OHRS="${OHRS:-ohrs}"
BUNDLE_NAME="${BUNDLE_NAME:-com.richerfu.paws}"
ABILITY_NAME="${ABILITY_NAME:-EntryAbility}"
HAP_PATH="${HAP_PATH:-$ROOT_DIR/entry/build/default/outputs/default/entry-default-unsigned.hap}"
HAP_LIB_PATH="${HAP_LIB_PATH:-libs/arm64-v8a/libpaws_ui.so}"
NAPI_DTS_PATH="${NAPI_DTS_PATH:-$ROOT_DIR/entry/src/main/cpp/types/libpaws_ui/Index.d.ts}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/smoke-logs}"
HILOG_SECONDS="${HILOG_SECONDS:-20}"
HDC_TARGET="${HDC_TARGET:-}"
RUN_BUILD=1
VERIFY_HAP_EXPORTS=1
FORCE_STOP_APP=1
PROFILE_PATH="${PROFILE_PATH:-}"
PROFILE_URL="${PROFILE_URL:-}"
PROFILE_NAME="${PROFILE_NAME:-Paws Smoke Profile}"
PROTOCOL_MODE="${PROTOCOL_MODE:-}"
MOCK_BIND="${MOCK_BIND:-127.0.0.1}"
MOCK_ADVERTISE_HOST="${MOCK_ADVERTISE_HOST:-}"
DELAY_PROXY="${DELAY_PROXY:-}"
DELAY_URL="${DELAY_URL:-}"
DELAY_TIMEOUT_MS="${DELAY_TIMEOUT_MS:-5000}"
EXPECT_DELAY_FAILURE="${EXPECT_DELAY_FAILURE:-0}"
ECHO_PROXY="${ECHO_PROXY:-}"
ECHO_URL="${ECHO_URL:-}"
ECHO_PAYLOAD="${ECHO_PAYLOAD:-paws-echo-payload}"
ECHO_TIMEOUT_MS="${ECHO_TIMEOUT_MS:-5000}"
EXPECT_ECHO_FAILURE="${EXPECT_ECHO_FAILURE:-0}"
DEVICE_PROBE_COMMAND="${DEVICE_PROBE_COMMAND:-}"
DEVICE_PROBE_MATCH="${DEVICE_PROBE_MATCH:-}"
EXPECT_DEVICE_PROBE_FAILURE="${EXPECT_DEVICE_PROBE_FAILURE:-0}"
REQUIRE_PROTECT_SUCCESS="${REQUIRE_PROTECT_SUCCESS:-0}"
AUTO_START_VPN=0
ALLOW_VPN_UNSUPPORTED=0

usage() {
  cat <<USAGE
Usage: scripts/harmony-smoke.sh [options]

Builds the aarch HAP, installs it with hdc, starts EntryAbility, and captures
hilog output for quick HarmonyOS device smoke validation.

Options:
  --no-build             Skip ohrs build.
  --hap PATH             Install this HAP instead of the default signed HAP.
  --skip-hap-export-check
                         Skip package check that HAP libpaws_ui.so contains
                         NAPI functions declared in Index.d.ts.
  --no-force-stop        Do not force-stop the app before install/launch.
  --target KEY           Pass an hdc target key, matching "hdc -t KEY ...".
  --hilog-seconds N      Seconds to capture hilog after launching the app.
  --profile PATH         Import this local YAML profile through debug Want args.
  --profile-url URL      Import this remote profile URL through debug Want args.
  --profile-name NAME    Name to use for an imported smoke profile.
  --protocol-mode MODE   Start local-protocol-tests MODE and import its profile.
  --mock-bind IP         Bind local-protocol-tests servers to this IP.
  --mock-advertise-host HOST
                         Host written into generated local-protocol profile.
  --delay-proxy NAME     Run debug proxy delay for this proxy after reload/start.
  --delay-url URL        URL to use for debug proxy delay.
  --delay-timeout-ms N   Timeout for debug proxy delay.
  --expect-delay-failure Treat proxy delay failure as the expected result.
  --echo-proxy NAME      Run debug TCP echo through this proxy.
  --echo-url URL         Echo server URL for debug TCP echo.
  --echo-payload TEXT    Payload expected to be echoed byte-for-byte.
  --echo-timeout-ms N    Timeout for debug TCP echo.
  --expect-echo-failure  Treat proxy echo failure as the expected result.
  --device-probe-command COMMAND
                         Run this hdc shell command after VPN startup.
  --device-probe-match REGEX
                         Require this regex in device probe output.
  --expect-device-probe-failure
                         Treat non-zero device probe exit as expected.
  --require-protect-success
                         Require Harmony process-network protection success.
  --auto-start-vpn       Request VPN start after profile import/reload.
  --allow-vpn-unsupported
                         Allow emulator-like environments where the VPN
                         extension start request is logged but no TUN is made.
  -h, --help             Show this help.

Environment overrides:
  HDC, OHRS, HDC_TARGET, BUNDLE_NAME, ABILITY_NAME, HAP_PATH, LOG_DIR,
  HAP_LIB_PATH, NAPI_DTS_PATH,
  HILOG_SECONDS, PROFILE_PATH, PROFILE_URL, PROFILE_NAME, PROTOCOL_MODE,
  MOCK_BIND, MOCK_ADVERTISE_HOST, DELAY_PROXY, DELAY_URL, DELAY_TIMEOUT_MS,
  EXPECT_DELAY_FAILURE, ECHO_PROXY, ECHO_URL, ECHO_PAYLOAD, ECHO_TIMEOUT_MS,
  EXPECT_ECHO_FAILURE, DEVICE_PROBE_COMMAND, DEVICE_PROBE_MATCH,
  EXPECT_DEVICE_PROBE_FAILURE, REQUIRE_PROTECT_SUCCESS
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-build)
      RUN_BUILD=0
      shift
      ;;
    --hap)
      HAP_PATH="${2:?missing HAP path}"
      shift 2
      ;;
    --skip-hap-export-check)
      VERIFY_HAP_EXPORTS=0
      shift
      ;;
    --no-force-stop)
      FORCE_STOP_APP=0
      shift
      ;;
    --target)
      HDC_TARGET="${2:?missing hdc target key}"
      shift 2
      ;;
    --hilog-seconds)
      HILOG_SECONDS="${2:?missing hilog seconds}"
      shift 2
      ;;
    --profile)
      PROFILE_PATH="${2:?missing profile path}"
      shift 2
      ;;
    --profile-url)
      PROFILE_URL="${2:?missing profile URL}"
      shift 2
      ;;
    --profile-name)
      PROFILE_NAME="${2:?missing profile name}"
      shift 2
      ;;
    --protocol-mode)
      PROTOCOL_MODE="${2:?missing protocol mode}"
      shift 2
      ;;
    --mock-bind)
      MOCK_BIND="${2:?missing mock bind IP}"
      shift 2
      ;;
    --mock-advertise-host)
      MOCK_ADVERTISE_HOST="${2:?missing mock advertise host}"
      shift 2
      ;;
    --delay-proxy)
      DELAY_PROXY="${2:?missing delay proxy}"
      shift 2
      ;;
    --delay-url)
      DELAY_URL="${2:?missing delay URL}"
      shift 2
      ;;
    --delay-timeout-ms)
      DELAY_TIMEOUT_MS="${2:?missing delay timeout ms}"
      shift 2
      ;;
    --expect-delay-failure)
      EXPECT_DELAY_FAILURE=1
      shift
      ;;
    --echo-proxy)
      ECHO_PROXY="${2:?missing echo proxy}"
      shift 2
      ;;
    --echo-url)
      ECHO_URL="${2:?missing echo URL}"
      shift 2
      ;;
    --echo-payload)
      ECHO_PAYLOAD="${2:?missing echo payload}"
      shift 2
      ;;
    --echo-timeout-ms)
      ECHO_TIMEOUT_MS="${2:?missing echo timeout ms}"
      shift 2
      ;;
    --expect-echo-failure)
      EXPECT_ECHO_FAILURE=1
      shift
      ;;
    --device-probe-command)
      DEVICE_PROBE_COMMAND="${2:?missing device probe command}"
      shift 2
      ;;
    --device-probe-match)
      DEVICE_PROBE_MATCH="${2:?missing device probe match regex}"
      shift 2
      ;;
    --expect-device-probe-failure)
      EXPECT_DEVICE_PROBE_FAILURE=1
      shift
      ;;
    --require-protect-success)
      REQUIRE_PROTECT_SUCCESS=1
      shift
      ;;
    --auto-start-vpn)
      AUTO_START_VPN=1
      shift
      ;;
    --allow-vpn-unsupported)
      ALLOW_VPN_UNSUPPORTED=1
      shift
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

shell_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/'\\\\''/g")"
}

force_stop_app() {
  if [ "$FORCE_STOP_APP" -eq 1 ]; then
    hdc_cmd shell aa force-stop "$BUNDLE_NAME" >/dev/null 2>&1 || true
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 127
  fi
}

verify_hap_native_exports() {
  if [ "$VERIFY_HAP_EXPORTS" -eq 0 ]; then
    return
  fi
  if [ ! -f "$NAPI_DTS_PATH" ]; then
    echo "NAPI declaration file not found: $NAPI_DTS_PATH" >&2
    echo "Pass --skip-hap-export-check to bypass this package guard." >&2
    exit 1
  fi

  mkdir -p "$LOG_DIR"
  declared_exports="$LOG_DIR/paws-napi-declared-$(date +%Y%m%d-%H%M%S).txt"
  missing_exports="$LOG_DIR/paws-napi-missing-$(date +%Y%m%d-%H%M%S).txt"
  hap_strings="$LOG_DIR/paws-hap-lib-strings-$(date +%Y%m%d-%H%M%S).txt"

  sed -n 's/^export declare function \([^(]*\)(.*/\1/p' "$NAPI_DTS_PATH" \
    | sort -u >"$declared_exports"
  if [ ! -s "$declared_exports" ]; then
    echo "No exported NAPI function declarations found in: $NAPI_DTS_PATH" >&2
    exit 1
  fi

  if ! unzip -p "$HAP_PATH" "$HAP_LIB_PATH" | strings >"$hap_strings"; then
    echo "Could not read $HAP_LIB_PATH from HAP: $HAP_PATH" >&2
    exit 1
  fi

  : >"$missing_exports"
  while IFS= read -r export_name; do
    if ! grep -F "$export_name" "$hap_strings" >/dev/null 2>&1; then
      printf "%s\n" "$export_name" >>"$missing_exports"
    fi
  done <"$declared_exports"

  if [ -s "$missing_exports" ]; then
    echo "HAP native library is missing NAPI exports declared in Index.d.ts:" >&2
    sed 's/^/  - /' "$missing_exports" >&2
    echo "HAP: $HAP_PATH" >&2
    echo "Library entry: $HAP_LIB_PATH" >&2
    echo "Expected declarations: $declared_exports" >&2
    echo "Extracted strings: $hap_strings" >&2
    echo "Rebuild/copy the latest libpaws_ui.so into entry/libs/arm64-v8a before signing." >&2
    exit 1
  fi
}

require_device() {
  if [ -n "$HDC_TARGET" ]; then
    hdc_cmd wait
    return
  fi

  targets=""
  attempts=0
  while [ "$attempts" -lt 5 ]; do
    targets="$("$HDC" list targets 2>/dev/null | awk 'NF && $1 !~ /^\[/ { print $1 }')"
    if [ -n "$targets" ]; then
      return
    fi
    attempts=$((attempts + 1))
    sleep 1
  done
  if [ -z "$targets" ]; then
    echo "No HarmonyOS device is connected. Connect a device or pass --target KEY." >&2
    "$HDC" list targets >&2 || true
    exit 1
  fi
}

mode_proxy_name() {
  case "$1" in
    direct) printf "%s" "DIRECT" ;;
    http) printf "%s" "HTTP-MOCK" ;;
    http-auth) printf "%s" "HTTP-AUTH-MOCK" ;;
    http-bad-auth) printf "%s" "HTTP-BAD-AUTH-MOCK" ;;
    http-down) printf "%s" "HTTP-DOWN-MOCK" ;;
    socks5) printf "%s" "SOCKS5-MOCK" ;;
    socks5-auth) printf "%s" "SOCKS5-AUTH-MOCK" ;;
    socks5-bad-auth) printf "%s" "SOCKS5-BAD-AUTH-MOCK" ;;
    ss) printf "%s" "SS-MOCK" ;;
    ss-bad-password) printf "%s" "SS-BAD-PASSWORD-MOCK" ;;
    trojan) printf "%s" "TROJAN-MOCK" ;;
    trojan-bad-password) printf "%s" "TROJAN-BAD-PASSWORD-MOCK" ;;
    vless) printf "%s" "VLESS-MOCK" ;;
    vless-bad-uuid) printf "%s" "VLESS-BAD-UUID-MOCK" ;;
    *) return 1 ;;
  esac
}

mode_expects_delay_failure() {
  case "$1" in
    http-bad-auth|http-down|socks5-bad-auth|ss-bad-password|trojan-bad-password|vless-bad-uuid) return 0 ;;
    *) return 1 ;;
  esac
}

expect_delay_failure_enabled() {
  [ "$EXPECT_DELAY_FAILURE" = "1" ] ||
    [ "$EXPECT_DELAY_FAILURE" = "true" ] ||
    [ "$EXPECT_DELAY_FAILURE" = "yes" ]
}

expect_echo_failure_enabled() {
  [ "$EXPECT_ECHO_FAILURE" = "1" ] ||
    [ "$EXPECT_ECHO_FAILURE" = "true" ] ||
    [ "$EXPECT_ECHO_FAILURE" = "yes" ]
}

expect_device_probe_failure_enabled() {
  [ "$EXPECT_DEVICE_PROBE_FAILURE" = "1" ] ||
    [ "$EXPECT_DEVICE_PROBE_FAILURE" = "true" ] ||
    [ "$EXPECT_DEVICE_PROBE_FAILURE" = "yes" ]
}

require_protect_success_enabled() {
  [ "$REQUIRE_PROTECT_SUCCESS" = "1" ] ||
    [ "$REQUIRE_PROTECT_SUCCESS" = "true" ] ||
    [ "$REQUIRE_PROTECT_SUCCESS" = "yes" ]
}

stop_protocol_lab() {
  if [ "${PROTOCOL_PID:-}" ]; then
    kill "$PROTOCOL_PID" >/dev/null 2>&1 || true
    wait "$PROTOCOL_PID" >/dev/null 2>&1 || true
  fi
}

start_protocol_lab() {
  mkdir -p "$LOG_DIR"
  PROFILE_PATH="$ROOT_DIR/local-protocol-tests/generated/$PROTOCOL_MODE.yaml"
  PROFILE_NAME="Paws $PROTOCOL_MODE Smoke"
  PROTOCOL_LOG_PATH="$LOG_DIR/local-protocol-$PROTOCOL_MODE-$(date +%Y%m%d-%H%M%S).log"
  rm -f "$PROFILE_PATH"

  set -- "$PROTOCOL_MODE" --profile-out "$PROFILE_PATH" --bind "$MOCK_BIND"
  if [ -n "$MOCK_ADVERTISE_HOST" ]; then
    set -- "$@" --advertise-host "$MOCK_ADVERTISE_HOST"
  fi

  (
    cd "$ROOT_DIR"
    cargo run --manifest-path local-protocol-tests/Cargo.toml -- "$@"
  ) >"$PROTOCOL_LOG_PATH" 2>&1 &
  PROTOCOL_PID=$!

  wait_seconds=0
  while [ "$wait_seconds" -lt 30 ]; do
    if [ -s "$PROFILE_PATH" ]; then
      if [ -z "$DELAY_PROXY" ]; then
        DELAY_PROXY="$(mode_proxy_name "$PROTOCOL_MODE")"
      fi
      if [ -z "$ECHO_PROXY" ]; then
        ECHO_PROXY="$(mode_proxy_name "$PROTOCOL_MODE")"
      fi
      if [ -z "$DELAY_URL" ]; then
        echo_target="$(sed -n 's/^# Echo target: //p' "$PROFILE_PATH" | head -n 1)"
        if [ -n "$echo_target" ]; then
          DELAY_URL="http://$echo_target"
        fi
      fi
      if [ -z "$ECHO_URL" ]; then
        echo_target="$(sed -n 's/^# Echo target: //p' "$PROFILE_PATH" | head -n 1)"
        if [ -n "$echo_target" ]; then
          ECHO_URL="http://$echo_target"
        fi
      fi
      if mode_expects_delay_failure "$PROTOCOL_MODE"; then
        EXPECT_DELAY_FAILURE=1
        EXPECT_ECHO_FAILURE=1
      fi
      return
    fi
    if ! kill -0 "$PROTOCOL_PID" >/dev/null 2>&1; then
      echo "local-protocol-tests exited before generating a profile." >&2
      echo "Protocol log saved to: $PROTOCOL_LOG_PATH" >&2
      exit 1
    fi
    sleep 1
    wait_seconds=$((wait_seconds + 1))
  done

  echo "Timed out waiting for local-protocol-tests profile generation." >&2
  echo "Protocol log saved to: $PROTOCOL_LOG_PATH" >&2
  exit 1
}

start_hilog_capture() {
  mkdir -p "$LOG_DIR"
  HILOG_PATH="$LOG_DIR/paws-smoke-$(date +%Y%m%d-%H%M%S).hilog"
  : >"$HILOG_PATH"
  hdc_cmd shell hilog -r >>"$HILOG_PATH" 2>&1 || true
  HILOG_CAPTURE_ACTIVE=1
}

stop_hilog_capture() {
  if [ "${HILOG_CAPTURE_ACTIVE:-}" ]; then
    hdc_cmd shell hilog -x >>"$HILOG_PATH" 2>&1 || true
    HILOG_CAPTURE_ACTIVE=""
  fi
}

cleanup() {
  stop_hilog_capture
  stop_protocol_lab
}

require_log_marker() {
  pattern="$1"
  description="$2"
  if ! grep -E "$pattern" "$HILOG_PATH" >/dev/null 2>&1; then
    echo "Smoke launched, but expected log marker was not found: $description" >&2
    echo "Hilog saved to: $HILOG_PATH" >&2
    if [ "${PROTOCOL_LOG_PATH:-}" ]; then
      echo "Protocol log saved to: $PROTOCOL_LOG_PATH" >&2
    fi
    exit 1
  fi
}

run_device_probe() {
  DEVICE_PROBE_PATH="$LOG_DIR/device-probe-$(date +%Y%m%d-%H%M%S).log"
  echo "Running device probe command: $DEVICE_PROBE_COMMAND"
  if hdc_cmd shell "$DEVICE_PROBE_COMMAND" >"$DEVICE_PROBE_PATH" 2>&1; then
    device_probe_status=0
  else
    device_probe_status=$?
  fi

  if expect_device_probe_failure_enabled; then
    if [ "$device_probe_status" -eq 0 ]; then
      echo "Device probe succeeded, but failure was expected." >&2
      echo "Device probe output saved to: $DEVICE_PROBE_PATH" >&2
      exit 1
    fi
    return
  fi

  if [ "$device_probe_status" -ne 0 ]; then
    echo "Device probe failed with exit status $device_probe_status." >&2
    echo "Device probe output saved to: $DEVICE_PROBE_PATH" >&2
    exit 1
  fi

  if [ -n "$DEVICE_PROBE_MATCH" ] &&
    ! grep -E "$DEVICE_PROBE_MATCH" "$DEVICE_PROBE_PATH" >/dev/null 2>&1; then
    echo "Device probe output did not match expected regex: $DEVICE_PROBE_MATCH" >&2
    echo "Device probe output saved to: $DEVICE_PROBE_PATH" >&2
    exit 1
  fi
}

trap cleanup EXIT
trap 'cleanup; exit 130' INT TERM

require_command "$HDC"
if [ "$VERIFY_HAP_EXPORTS" -eq 1 ]; then
  require_command unzip
  require_command strings
fi
if [ "$RUN_BUILD" -eq 1 ]; then
  require_command "$OHRS"
fi
if [ -n "$PROTOCOL_MODE" ]; then
  require_command cargo
fi

profile_source_count=0
if [ -n "$PROFILE_PATH" ]; then
  profile_source_count=$((profile_source_count + 1))
fi
if [ -n "$PROFILE_URL" ]; then
  profile_source_count=$((profile_source_count + 1))
fi
if [ -n "$PROTOCOL_MODE" ]; then
  profile_source_count=$((profile_source_count + 1))
fi
if [ "$profile_source_count" -gt 1 ]; then
  echo "Use only one of --profile, --profile-url, or --protocol-mode." >&2
  exit 2
fi

require_device

if [ -n "$PROTOCOL_MODE" ]; then
  start_protocol_lab
fi

PROFILE_CONTENT_BASE64=""
if [ -n "$PROFILE_PATH" ]; then
  if [ ! -f "$PROFILE_PATH" ]; then
    echo "Profile not found: $PROFILE_PATH" >&2
    exit 1
  fi
  PROFILE_CONTENT_BASE64="$(base64 <"$PROFILE_PATH" | tr -d '\n')"
fi

if [ "$RUN_BUILD" -eq 1 ]; then
  (cd "$ROOT_DIR" && "$OHRS" build --arch aarch)
fi

if [ ! -f "$HAP_PATH" ]; then
  echo "HAP not found: $HAP_PATH" >&2
  exit 1
fi
verify_hap_native_exports

start_hilog_capture

force_stop_app
hdc_cmd install -r "$HAP_PATH"
force_stop_app
AA_ARGS="aa start -b $(shell_quote "$BUNDLE_NAME") -a $(shell_quote "$ABILITY_NAME")"
if [ -n "$PROFILE_CONTENT_BASE64" ]; then
  AA_ARGS="$AA_ARGS --ps pawsProfileContentBase64 $(shell_quote "$PROFILE_CONTENT_BASE64") --ps pawsProfileName $(shell_quote "$PROFILE_NAME")"
elif [ -n "$PROFILE_URL" ]; then
  AA_ARGS="$AA_ARGS --ps pawsProfileUrl $(shell_quote "$PROFILE_URL") --ps pawsProfileName $(shell_quote "$PROFILE_NAME")"
fi
if [ "$AUTO_START_VPN" -eq 1 ]; then
  AA_ARGS="$AA_ARGS --ps pawsAutoStartVpn true"
fi
if [ -n "$DELAY_PROXY" ]; then
  AA_ARGS="$AA_ARGS --ps pawsDelayProxy $(shell_quote "$DELAY_PROXY") --ps pawsDelayTimeoutMs $(shell_quote "$DELAY_TIMEOUT_MS")"
  if [ -n "$DELAY_URL" ]; then
    AA_ARGS="$AA_ARGS --ps pawsDelayUrl $(shell_quote "$DELAY_URL")"
  fi
  if expect_delay_failure_enabled; then
    AA_ARGS="$AA_ARGS --ps pawsExpectDelayFailure true"
  fi
fi
if [ -n "$ECHO_PROXY" ]; then
  AA_ARGS="$AA_ARGS --ps pawsEchoProxy $(shell_quote "$ECHO_PROXY") --ps pawsEchoPayload $(shell_quote "$ECHO_PAYLOAD") --ps pawsEchoTimeoutMs $(shell_quote "$ECHO_TIMEOUT_MS")"
  if [ -n "$ECHO_URL" ]; then
    AA_ARGS="$AA_ARGS --ps pawsEchoUrl $(shell_quote "$ECHO_URL")"
  fi
  if expect_echo_failure_enabled; then
    AA_ARGS="$AA_ARGS --ps pawsExpectEchoFailure true"
  fi
fi
hdc_cmd shell "$AA_ARGS"
sleep "$HILOG_SECONDS"
if [ -n "$DEVICE_PROBE_COMMAND" ]; then
  run_device_probe
fi

stop_hilog_capture

require_log_marker 'PawsEntry|PawsVpn|paws core|meow-rs' 'Paws app/core markers'
if [ -n "$PROFILE_CONTENT_BASE64" ] || [ -n "$PROFILE_URL" ]; then
  require_log_marker 'debug automation profile ready' 'debug profile import/reload completed'
fi
if [ "$AUTO_START_VPN" -eq 1 ]; then
  require_log_marker 'debug automation requested VPN start' 'debug automation requested VPN start'
  if [ "$ALLOW_VPN_UNSUPPORTED" -eq 1 ]; then
    require_log_marker 'request VPN start received|requested VPN start|debug automation requested VPN start' 'VPN start request was issued'
  else
    require_log_marker 'created tun fd [0-9]+' 'Harmony VPN TUN was created'
    if require_protect_success_enabled; then
      require_log_marker 'protected process network' 'Harmony VPN egress protection succeeded'
    else
      require_log_marker 'protected process network|protect process network failed' 'Harmony VPN egress protection result'
    fi
  fi
fi
if [ -n "$DELAY_PROXY" ]; then
  if expect_delay_failure_enabled; then
    require_log_marker 'debug automation delay failed as expected' 'debug proxy delay failed as expected'
  else
    require_log_marker 'debug automation delay result' 'debug proxy delay succeeded'
  fi
fi
if [ -n "$ECHO_PROXY" ]; then
  if expect_echo_failure_enabled; then
    require_log_marker 'debug automation echo failed as expected' 'debug proxy echo failed as expected'
  else
    require_log_marker 'debug automation echo result' 'debug proxy echo succeeded'
  fi
fi

echo "HarmonyOS smoke passed."
echo "Hilog saved to: $HILOG_PATH"
if [ "${PROTOCOL_LOG_PATH:-}" ]; then
  echo "Protocol log saved to: $PROTOCOL_LOG_PATH"
fi
if [ "${DEVICE_PROBE_PATH:-}" ]; then
  echo "Device probe output saved to: $DEVICE_PROBE_PATH"
fi

#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
SMOKE_SCRIPT="${SMOKE_SCRIPT:-$ROOT_DIR/scripts/harmony-smoke.sh}"
MODES="${MODES:-direct http http-auth http-bad-auth http-down socks5 socks5-auth socks5-bad-auth ss ss-bad-password trojan trojan-bad-password vless vless-bad-uuid}"
MOCK_BIND="${MOCK_BIND:-0.0.0.0}"
MOCK_ADVERTISE_HOST="${MOCK_ADVERTISE_HOST:-}"
HILOG_SECONDS="${HILOG_SECONDS:-20}"
REQUIRE_PROTECT_SUCCESS="${REQUIRE_PROTECT_SUCCESS:-1}"
RUN_BUILD_ONCE="${RUN_BUILD_ONCE:-1}"
ALLOW_VPN_UNSUPPORTED="${ALLOW_VPN_UNSUPPORTED:-0}"

usage() {
  cat <<USAGE
Usage: scripts/harmony-protocol-matrix.sh [options]

Runs scripts/harmony-smoke.sh for every local-protocol-tests mode, importing the
generated profile, requesting VPN start, and validating protect/delay/echo logs.

Options:
  --modes "A B"              Space-separated protocol modes to run.
  --mock-bind IP             Bind local-protocol-tests servers to this IP.
  --mock-advertise-host HOST Host written into generated profiles.
  --hilog-seconds N          Seconds to capture hilog per mode.
  --no-require-protect-success
                              Accept explicit protect failure as diagnostics.
  --no-build-once            Let every smoke invocation run its own build.
  --allow-vpn-unsupported    Forward emulator allowance to harmony-smoke.
  -h, --help                 Show this help.

Most harmony-smoke.sh environment overrides still apply, including HDC,
HDC_TARGET, HAP_PATH, BUNDLE_NAME, LOG_DIR, and OHRS.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --modes)
      MODES="${2:?missing modes}"
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
    --hilog-seconds)
      HILOG_SECONDS="${2:?missing hilog seconds}"
      shift 2
      ;;
    --no-require-protect-success)
      REQUIRE_PROTECT_SUCCESS=0
      shift
      ;;
    --no-build-once)
      RUN_BUILD_ONCE=0
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

if [ ! -x "$SMOKE_SCRIPT" ]; then
  echo "Smoke script is not executable: $SMOKE_SCRIPT" >&2
  exit 1
fi

if [ "$RUN_BUILD_ONCE" -eq 1 ]; then
  (cd "$ROOT_DIR" && "${OHRS:-ohrs}" build --arch aarch)
  no_build_arg="--no-build"
else
  no_build_arg=""
fi

protect_arg=""
if [ "$REQUIRE_PROTECT_SUCCESS" -eq 1 ]; then
  protect_arg="--require-protect-success"
fi

allow_arg=""
if [ "$ALLOW_VPN_UNSUPPORTED" -eq 1 ]; then
  allow_arg="--allow-vpn-unsupported"
fi

for mode in $MODES; do
  echo "Running Harmony protocol smoke: $mode"
  set -- --protocol-mode "$mode" --auto-start-vpn \
    --mock-bind "$MOCK_BIND" \
    --hilog-seconds "$HILOG_SECONDS"
  if [ -n "$MOCK_ADVERTISE_HOST" ]; then
    set -- "$@" --mock-advertise-host "$MOCK_ADVERTISE_HOST"
  fi
  if [ -n "$no_build_arg" ]; then
    set -- "$@" "$no_build_arg"
  fi
  if [ -n "$protect_arg" ]; then
    set -- "$@" "$protect_arg"
  fi
  if [ -n "$allow_arg" ]; then
    set -- "$@" "$allow_arg"
  fi
  "$SMOKE_SCRIPT" "$@"
done

echo "Harmony protocol matrix passed: $MODES"

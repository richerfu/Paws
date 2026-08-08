# Paws App Protocol Acceptance Matrix

This matrix is the manual App-side regression flow for the local protocol lab in
`local-protocol-tests`. It is meant to be run after changes to VPN startup,
DNS handling, provider refresh, profile parsing, proxy selection, or meow-rs
integration.

## Setup

Run the app build first:

```bash
ohrs build --arch aarch
```

For a quick device launch smoke before the manual protocol matrix, run:

```bash
scripts/harmony-smoke.sh
```

After generating a local protocol profile, the smoke script can also import it
and request VPN start:

```bash
scripts/harmony-smoke.sh --profile local-protocol-tests/generated/direct.yaml --auto-start-vpn
```

For a one-command app smoke that starts the local protocol server, generates the
profile, imports it, and requests VPN start:

```bash
scripts/harmony-smoke.sh --protocol-mode http --auto-start-vpn --mock-bind 0.0.0.0 --mock-advertise-host 192.168.1.23
```

To run every supported local protocol mode with the same install/start/protect/
delay/echo assertions, use the matrix wrapper:

```bash
scripts/harmony-protocol-matrix.sh --mock-advertise-host 192.168.1.23
```

That smoke asserts the app launched, the profile import/reload completed, the
VPN TUN fd was created, and the Harmony process-network protection path
reported success or an explicit failure in `hilog`. It also runs the generated
proxy delay check; positive modes must produce `debug automation delay result`,
and `debug automation echo result`, while negative modes must produce
`debug automation delay failed as expected` and
`debug automation echo failed as expected`.

For PAWS-MEOW-001 egress-protection acceptance, add
`--require-protect-success` to require a successful `protectProcessNet()` log
instead of accepting an explicit protection failure as diagnostic evidence.

For process-external traffic acceptance, add `--device-probe-command` with a
device-side HTTP/TCP command that reaches the printed echo URL, and optionally
`--device-probe-match` for the expected payload. The command runs through
`hdc shell`, outside the Paws process, so it is useful for checking whether
non-Paws traffic traverses the active VPN TUN on a given test device.

Start one local protocol server from the repository root and keep it running
while testing:

```bash
cargo run --manifest-path local-protocol-tests/Cargo.toml -- http
```

For a physical device, bind the mock servers to all interfaces and advertise
the Mac LAN IP:

```bash
cargo run --manifest-path local-protocol-tests/Cargo.toml -- http --bind 0.0.0.0 --advertise-host 192.168.1.23
```

Use the generated profile printed by the command:

```text
local-protocol-tests/generated/<mode>.yaml
```

The command also prints the proxy name and the native delay URL:

```text
proxy=<PROXY_NAME>, url=http://<echo-host>:<echo-port>
```

## Common App Flow

Run this sequence for each mode in the matrix:

1. Start `local-protocol-tests` for the mode and keep the process alive.
2. Import `local-protocol-tests/generated/<mode>.yaml` in the Profiles page.
3. Activate the imported profile.
4. Start VPN from the Dashboard.
5. Confirm Dashboard shows VPN connected, controller started, engine loaded,
   and outbound protection is either protected or has an explicit failure.
6. Open Proxies and select the generated proxy in group `Proxy` when the mode
   has a proxy. For `direct`, select `DIRECT`.
7. Run delay test for the generated proxy. The expected result is a finite
   latency for positive modes.
8. Open Traffic and confirm TUN or meow-rs counters move after the delay test.
9. Open Logs and confirm there are no panic-level or repeated dial errors for
   positive modes.
10. Stop VPN, then stop the local protocol process.

If an echo-payload client is available on the device, send a small TCP payload
to the printed echo target while VPN is connected. Expected result for positive
modes is byte-for-byte echo. For `http-bad-auth`, `http-down`,
`socks5-bad-auth`, `trojan-bad-password`, and `vless-bad-uuid`, the payload
must fail cleanly without crashing the app or leaving VPN stuck in a connecting
state.

## Matrix

Latest current-revision simulator result: on 2026-07-16, the full 14-mode
matrix passed profile import/reload plus native meow delay/TCP echo checks for
all positive and expected-failure modes. The simulator did not launch the VPN
extension or create a TUN, so that result is not a system-VPN pass. The current
revision still requires the physical-device TUN/protect/external-process flow
documented in `docs/integration-verification-2026-07-16.md`.

Latest physical-device positive smoke: on 2026-05-24, target
`2PM0223914001038`, signed HAP
`entry/build/default/outputs/default/entry-default-signed.hap`, host
`192.168.3.28`, the automated smoke passed for `direct`, `http`, `http-auth`,
`socks5`, `socks5-auth`, `ss`, `trojan`, and `vless` with
`--auto-start-vpn --require-protect-success`. Each run imported the generated
profile, requested VPN start, created a TUN fd, reported
`protected process network`, and completed native delay plus TCP echo.

| Mode | Profile | Proxy name | Expected import/reload | Expected VPN start | Expected delay | Expected echo payload |
| --- | --- | --- | --- | --- | --- | --- |
| `direct` | `generated/direct.yaml` | `DIRECT` | Imports, activates, proxy group contains `DIRECT` | VPN connects, DNS diagnostics visible | Succeeds against printed echo URL | Echo roundtrip succeeds without mock proxy |
| `http` | `generated/http.yaml` | `HTTP-MOCK` | Imports, activates, proxy group contains `HTTP-MOCK` and `DIRECT` | VPN connects through HTTP CONNECT mock | Succeeds | Echo roundtrip succeeds through HTTP CONNECT |
| `http-auth` | `generated/http-auth.yaml` | `HTTP-AUTH-MOCK` | Imports with username/password and custom header | VPN connects through authenticated HTTP CONNECT mock | Succeeds | Echo roundtrip succeeds; mock rejects if auth/header are missing |
| `http-bad-auth` | `generated/http-bad-auth.yaml` | `HTTP-BAD-AUTH-MOCK` | Imports and activates so the negative path is testable | VPN starts, but HTTP CONNECT auth should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |
| `http-down` | `generated/http-down.yaml` | `HTTP-DOWN-MOCK` | Imports and activates so the unavailable-proxy path is testable | VPN starts, but proxy TCP connect should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |
| `socks5` | `generated/socks5.yaml` | `SOCKS5-MOCK` | Imports, activates, proxy group contains `SOCKS5-MOCK` | VPN connects through SOCKS5 no-auth mock | Succeeds | Echo roundtrip succeeds through SOCKS5 |
| `socks5-auth` | `generated/socks5-auth.yaml` | `SOCKS5-AUTH-MOCK` | Imports with username/password | VPN connects through SOCKS5 username/password mock | Succeeds | Echo roundtrip succeeds; mock rejects bad credentials |
| `socks5-bad-auth` | `generated/socks5-bad-auth.yaml` | `SOCKS5-BAD-AUTH-MOCK` | Imports and activates so the negative path is testable | VPN starts, but SOCKS5 auth should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |
| `ss` | `generated/ss.yaml` | `SS-MOCK` | Imports with `aes-128-gcm` and password | VPN connects through Shadowsocks AEAD TCP mock | Succeeds | Echo roundtrip succeeds through Shadowsocks |
| `ss-bad-password` | `generated/ss-bad-password.yaml` | `SS-BAD-PASSWORD-MOCK` | Imports and activates so the negative path is testable | VPN starts, but Shadowsocks password validation should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |
| `trojan` | `generated/trojan.yaml` | `TROJAN-MOCK` | Imports with TLS and password | VPN connects through Trojan TLS mock | Succeeds | Echo roundtrip succeeds through Trojan |
| `trojan-bad-password` | `generated/trojan-bad-password.yaml` | `TROJAN-BAD-PASSWORD-MOCK` | Imports and activates so the negative path is testable | VPN starts, but Trojan password validation should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |
| `vless` | `generated/vless.yaml` | `VLESS-MOCK` | Imports with expected UUID | VPN connects through VLESS mock | Succeeds | Echo roundtrip succeeds through VLESS |
| `vless-bad-uuid` | `generated/vless-bad-uuid.yaml` | `VLESS-BAD-UUID-MOCK` | Imports and activates so the negative path is testable | VPN starts, but proxy handshake should fail cleanly | Fails or times out with a visible error | Echo payload fails cleanly; app remains responsive |

## Failure Checks

Use these checks when a positive mode fails:

- Profiles: the imported profile is active and the selected proxy persists
  after leaving and returning to the page.
- Dashboard: `engineLoaded` is true after activation; `vpnRunning` is true only
  after VPN start.
- Dashboard: outbound protection is not silently unknown after VPN creation.
- Settings DNS: model, listen address, TUN DNS address, upstreams, and
  intercepted query count are visible.
- Traffic: TUN or meow-rs counters change after delay or echo traffic.
- Connections: active connection entries show host/network/rule/proxy while a
  long-running echo payload is open.
- Logs: errors name the failed proxy/provider/DNS action instead of only
  reporting a generic failure.

## Smoke All Profile Generation

This does not exercise the app, but verifies every local mock profile can still
be generated:

```bash
scripts/verify-local-protocols.sh
```

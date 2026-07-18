# Local Protocol Tests

This directory is a manual protocol lab for the HMeta app.

It starts local echo/mock servers on random loopback ports, generates a Clash YAML profile with those ports filled in, and keeps the servers alive while you import that profile into the app.

## Run

From the repository root:

```bash
cargo run --manifest-path local-protocol-tests/Cargo.toml -- http
```

Supported modes:

```text
direct
http
http-auth
http-bad-auth
http-down
socks5
socks5-auth
socks5-bad-auth
ss
ss-bad-password
trojan
trojan-bad-password
vless
vless-bad-uuid
```

The tool writes a generated profile under:

```text
local-protocol-tests/generated/<mode>.yaml
```

Keep the process running while testing. Stop it with `Ctrl-C`.

For a physical device, bind to all interfaces and advertise your Mac's LAN IP:

```bash
DEFAULT_IF=$(route -n get default | awk '/interface:/{print $2}')
MAC_IP=$(ipconfig getifaddr "$DEFAULT_IF")
cargo run --manifest-path local-protocol-tests/Cargo.toml -- http --bind 0.0.0.0 --advertise-host "$MAC_IP"
```

Do not use `networksetup -getinfo Wi-Fi | awk '/IP address:/{print $3}'` here:
that pattern also matches the `IPv6 IP address:` line and can inject a newline
into the generated YAML. The generator rejects multiline host values and
validates the rendered YAML before writing it.

The HarmonyOS smoke script can start the same mode, import the generated
profile, request VPN start, and run the generated proxy delay plus TCP echo
checks in one run:

```bash
scripts/harmony-smoke.sh --protocol-mode http --auto-start-vpn --mock-bind 0.0.0.0 --mock-advertise-host 192.168.1.23
```

For host-side or emulator loopback checks, the default `127.0.0.1` is fine.

For short smoke runs:

```bash
cargo run --manifest-path local-protocol-tests/Cargo.toml -- http --keepalive-ms 1000
```

## Manual App Flow

The full App-side acceptance matrix lives in
`docs/app-protocol-acceptance.md`.

1. Run one mode, for example `http`.
2. Import `local-protocol-tests/generated/http.yaml` into the app.
3. Activate the imported profile.
4. Reload/start VPN.
5. Test the proxy delay for the generated proxy name, using the printed echo URL when calling native/API-level tests.

Proxy names:

```text
direct         -> DIRECT
http           -> HTTP-MOCK
http-auth      -> HTTP-AUTH-MOCK
http-bad-auth  -> HTTP-BAD-AUTH-MOCK
http-down      -> HTTP-DOWN-MOCK
socks5         -> SOCKS5-MOCK
socks5-auth    -> SOCKS5-AUTH-MOCK
socks5-bad-auth -> SOCKS5-BAD-AUTH-MOCK
ss             -> SS-MOCK
ss-bad-password -> SS-BAD-PASSWORD-MOCK
trojan         -> TROJAN-MOCK
trojan-bad-password -> TROJAN-BAD-PASSWORD-MOCK
vless          -> VLESS-MOCK
vless-bad-uuid -> VLESS-BAD-UUID-MOCK
```

The generated YAML contains comments with the echo target and proxy port.

`http-bad-auth`, `socks5-bad-auth`, `ss-bad-password`,
`trojan-bad-password`, `http-down`, and `vless-bad-uuid` are negative-path
profiles. The auth mock servers require the same credentials/UUID/password as
the positive profiles while the generated YAML intentionally sends invalid
values; `http-down` points at an unused local port. Use them to confirm failed
handshakes and connection failures are surfaced cleanly.

## Smoke All Profiles

This only verifies the local mock servers can start and generate profiles:

```bash
scripts/verify-local-protocols.sh
```

## Shape

The flow mirrors the upstream `meow-rs` integration style:

```text
app/profile -> meow-rs config -> proxy adapter -> local mock proxy -> local echo target
```

For `direct`, there is no mock proxy. The profile routes loopback traffic directly to the local echo target.

Current coverage follows the latest `meow-rs` protocol tests that use
embedded servers: HTTP CONNECT, HTTP CONNECT with Basic auth and custom headers,
SOCKS5 no-auth, SOCKS5 username/password auth, Shadowsocks AEAD TCP,
Trojan over TLS with SHA-224 password validation, and VLESS UUID validation.

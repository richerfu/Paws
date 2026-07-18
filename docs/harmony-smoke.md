# HarmonyOS Smoke Validation

This smoke check is the first automation layer for HMETA-MEOW-014. It covers
the device loop that is easy to regress during VPN and native binding work:

- build aarch HAP
- install HAP through `hdc`
- launch `EntryAbility`
- optionally start `local-protocol-tests` and import its generated profile
- optionally request VPN start
- capture `hilog`
- verify HMeta log markers are present, including TUN/protection markers when
  VPN start is requested

Run it with a HarmonyOS device connected:

```sh
scripts/harmony-smoke.sh
```

Useful variants:

```sh
scripts/harmony-smoke.sh --target <hdc-target-key>
scripts/package-hap.sh
scripts/harmony-smoke.sh --no-build --hap entry/build/default/outputs/default/entry-default-unsigned.hap
scripts/harmony-smoke.sh --hilog-seconds 45
scripts/harmony-smoke.sh --profile local-protocol-tests/generated/direct.yaml --auto-start-vpn
scripts/harmony-smoke.sh --profile local-protocol-tests/generated/direct.yaml --auto-start-vpn --require-protect-success
scripts/harmony-smoke.sh --protocol-mode http --auto-start-vpn --mock-bind 0.0.0.0 --mock-advertise-host 192.168.1.23
scripts/harmony-protocol-matrix.sh --mock-advertise-host 192.168.1.23
scripts/harmony-protocol-matrix.sh --allow-vpn-unsupported --no-require-protect-success --mock-advertise-host 192.168.1.23
scripts/harmony-subscription-ui-smoke.sh
scripts/harmony-settings-ui-smoke.sh
scripts/harmony-smoke.sh --protocol-mode direct --auto-start-vpn --device-probe-command 'toybox wget -q -O - http://192.168.1.23:12345' --device-probe-match hmeta
scripts/harmony-smoke.sh --profile local-protocol-tests/generated/http.yaml --delay-proxy HTTP-MOCK --delay-url http://192.168.1.23:12345
scripts/harmony-smoke.sh --profile-url https://example.test/profile.yaml --profile-name RemoteSmoke
```

The script writes logs to `smoke-logs/`. When it fails, inspect the generated
`.hilog` file first; it should contain `HMetaEntry`, `HMetaVpn`, `hmeta core`,
or `meow-rs` markers after a successful launch.

The current default package for simulator/local validation is unsigned. Use
`scripts/package-hap.sh` to run `ohrs build --arch aarch`, copy the latest
native library into the entry module, and package
`entry-default-unsigned.hap` through hvigor without signing.

Before installing and launching, the script force-stops the app by default so
each protocol run starts from a fresh EntryAbility/VpnExtensionAbility process.
Pass `--no-force-stop` only when intentionally testing `onNewWant` handling for
an already running app.

The script also checks the HAP before install: `libs/arm64-v8a/libhmeta_ui.so`
must contain every NAPI function declared in
`entry/src/main/cpp/types/libhmeta_ui/Index.d.ts`. This catches stale native
libraries in signed packages before they reach the device. Pass
`--skip-hap-export-check` only for diagnosing package contents manually.

When `--protocol-mode` is used, the script also writes a
`local-protocol-<mode>-*.log` file and keeps the mock server alive until the
smoke exits. For a physical device, pass a device-reachable host with
`--mock-bind 0.0.0.0 --mock-advertise-host <Mac-LAN-IP>` so the generated
profile does not point the device back at its own loopback address. The script
also infers the generated proxy name and echo URL, runs debug proxy delay plus
a byte-for-byte TCP echo roundtrip, and expects failure automatically for
negative modes such as `http-bad-auth`, `http-down`, `socks5-bad-auth`,
`ss-bad-password`, `trojan-bad-password`, and `vless-bad-uuid`.

## Debug Automation Want Parameters

`EntryAbility` accepts these smoke-only Want parameters:

- `hmetaProfileContent`: raw YAML/profile content.
- `hmetaProfileContentBase64`: base64 UTF-8 profile content. This is what
  `scripts/harmony-smoke.sh --profile` uses because it is stable through
  `hdc shell aa start` argument parsing.
- `hmetaProfileContentEscaped`: profile content with `\n` line escapes,
  retained for compatibility.
- `hmetaProfileUrl`: remote profile URL.
- `hmetaProfileName`: optional imported profile name.
- `hmetaAutoStartVpn`: `true`, `1`, or `yes` to request VPN start after import.
- `hmetaDelayProxy`: proxy name used for a debug delay check.
- `hmetaDelayUrl`: optional URL used for that debug delay check.
- `hmetaDelayTimeoutMs`: optional delay timeout in milliseconds.
- `hmetaExpectDelayFailure`: `true`, `1`, or `yes` when delay failure is the
  expected result.
- `hmetaEchoProxy`: proxy name used for a debug TCP echo roundtrip.
- `hmetaEchoUrl`: echo server URL for the debug TCP echo roundtrip.
- `hmetaEchoPayload`: optional UTF-8 payload expected to be echoed
  byte-for-byte.
- `hmetaEchoTimeoutMs`: optional echo timeout in milliseconds.
- `hmetaExpectEchoFailure`: `true`, `1`, or `yes` when echo failure is the
  expected result.

`scripts/harmony-smoke.sh` also accepts `--device-probe-command` to run an
arbitrary `hdc shell` command after the app launch/VPN settle window. Use
`--device-probe-match` to require a regex in the command output, or
`--expect-device-probe-failure` for negative modes. The probe runs outside the
HMeta process, so it is the first automation hook for checking whether device
traffic from another process traverses the active VPN.

The script uses these parameters when `--profile`, `--profile-url`, and
`--auto-start-vpn` are passed. It also sets the delay and echo parameters
automatically for `--protocol-mode`.

Pass `--require-protect-success` when validating HMETA-MEOW-001 on a device.
Without it, the smoke only requires an explicit process-network protection
result, so devices that log `protect process network failed` still preserve
diagnostic evidence instead of failing before hilog is saved.

For a full supported-protocol regression, use
`scripts/harmony-protocol-matrix.sh`. It loops through the local protocol modes,
starts each mock server, imports the generated profile, requests VPN start, and
checks protect, delay, and TCP echo logs. It builds once by default and then
passes `--no-build` to each smoke run. On physical devices, pass
`--mock-advertise-host <LAN-IP>` so generated profiles point at a reachable
host. The matrix requires `protectProcessNet()` success by default; pass
`--no-require-protect-success` only when collecting diagnostic evidence on a
device whose process-network protection behavior is still under investigation.

`--allow-vpn-unsupported` is only for simulator diagnostics. The Entry ability
still issues the system VPN request, but the smoke does not claim that a TUN was
created. A request that does not resolve within 15 seconds is persisted as a
failed lifecycle state; the Dashboard must remain disconnected and display the
startup failure. Do not use this flag for physical-device release acceptance.

The UI-specific smoke scripts cover interactions that hilog cannot validate:

- `scripts/harmony-subscription-ui-smoke.sh`: imported subscription card,
  overflow action set, and edit form.
- `scripts/harmony-settings-ui-smoke.sh`: settings row alignment, isolated
  route scroll state, hidden bottom navigation on secondary pages, About long
  text containment, privacy-row alignment, and repository-link alignment.

See `docs/integration-verification-2026-07-16.md` for the current simulator
evidence and the exact remaining physical-device flow.

## Current Scope

The script can now drive install, launch, profile import/reload, and VPN start
request. With `--auto-start-vpn`, it requires `HMetaVpn` logs for TUN creation
and either successful process-network protection or an explicit protection
failure; `--require-protect-success` tightens that to successful protection
only. With `--protocol-mode`, it can start the local mock profile generator
itself, feed the generated YAML into debug automation, and run the native proxy
delay and TCP echo paths against the generated echo server.

For third-party/process-external traffic, the script can run a supplied
`hdc shell` probe command after VPN startup and assert success, failure, or an
output regex. The remaining E2E work is to standardize a device helper or
portable shell command for every supported test device:

1. add an on-device helper or target command that sends traffic through the VPN TUN
2. assert positive modes echo payloads from that app path
3. assert negative modes fail cleanly without leaving VPN stuck

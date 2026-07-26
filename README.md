# Paws

Paws is a Clash/mihomo HarmonyOS client with a native ArkUI interface, powered
by [meow-rs](https://github.com/madeye/meow-rs) and a Rust userspace TUN stack.
Its product flow follows
[Meow for Android](https://github.com/madeye/meow), while the HarmonyOS UI,
VPN lifecycle, native bridge, packaging, and device validation are implemented
in this repository.

> Paws is under active development. Validate system VPN behavior on a physical
> device or a standard VPN-enabled OpenHarmony QEMU image. A DevEco simulator
> can validate UI and profile flows, but does not prove that a TUN was created.

## Architecture

```text
Native ArkUI (Rust: arkit + Dioxus)
    |  N-API
    v
HarmonyOS abilities (ArkTS)
    |  EntryAbility + VpnExtensionAbility
    |  VpnConnection TUN + protectProcessNet()
    v
Paws Rust runtime
    |  hmeta_core + hmeta_profile
    |  hmeta_vpn + netstack-smoltcp/lwIP
    v
meow-rs
    |  Config, rule engine, proxy, DNS, external controller
    v
Network
```

## Features

- **Routing modes**
  - Rule: follow the active profile's ordered rules and rule providers.
  - Global: send all eligible traffic through the selected proxy.
  - Direct: bypass proxies.
- **Proxy protocols**: Direct, Shadowsocks, Trojan, VLESS, VMess, Snell,
  Hysteria2, AnyTLS, HTTP, and SOCKS5 through the enabled meow-rs feature set.
- **Profiles and subscriptions**
  - Import Clash YAML, base64 subscriptions, local files, remote URLs, and
    common share links.
  - Refresh, activate, edit, export, restore, and delete profiles.
  - Preserve subscription usage, expiry, update interval, home, and support
    metadata when provided.
- **Rules and resources**: ordered rules, rule providers, proxy providers,
  provider refresh, GEOIP, GEOSITE, and ASN data.
- **VPN and DNS**: HarmonyOS `VpnExtensionAbility`, userspace TCP/UDP forwarding,
  in-TUN DNS handling, cache diagnostics, and process-network protection.
- **Runtime views**: proxy selection and delay test, live traffic, connections,
  requests, logs, resources, and VPN lifecycle diagnostics.
- **Native UI**: phone, tablet, and 2-in-1 device targets built with
  [arkit](https://github.com/richerfu/arkit).
- **Localization**: English and Simplified Chinese.
- **Automation**: workspace tests, local protocol mocks, HAP export checks,
  device/QEMU smoke tests, and a 14-mode positive/negative protocol matrix.

## Building

### Prerequisites

- macOS with DevEco Studio and the HarmonyOS SDK installed.
- Rust 1.89 or a compatible newer toolchain.
- [`ohrs`](https://github.com/ohos-rs/ohos-rs) available in `PATH`.
- `hvigorw` and `hdc`; the packaging scripts can use the copies bundled with
  DevEco Studio.
- Signing material configured locally when producing a physical-device HAP.

The project targets HarmonyOS 6.1, is compatible with HarmonyOS 6.0.2, and
currently packages the `arm64-v8a` native library.

### Verify the source tree

```sh
scripts/verify.sh
```

This runs Rust formatting checks, workspace tests, local protocol profile
generation, and an OpenHarmony `aarch` build.

### Build an unsigned release HAP

Provision the GeoData files required by GEOIP, GEOSITE, and ASN rules, then
package the app:

```sh
scripts/fetch-geodata.sh
scripts/package-hap.sh
```

Output:

```text
entry/build/default/outputs/default/entry-default-unsigned.hap
```

### Build a signed release HAP

After configuring a valid HarmonyOS signing profile for the target device:

```sh
scripts/fetch-geodata.sh
scripts/package-signed-hap.sh
```

Output:

```text
entry/build/default/outputs/default/entry-default-release-signed.hap
```

Do not unpack and rebuild a signed HAP with a generic ZIP utility. Doing so can
change the aligned resource layout and cause application icon or launcher
problems on physical devices.

See [CI and release checks](docs/ci-and-release.md) and
[GeoData provisioning](docs/geodata.md) for details.

## Install and launch

List connected targets:

```sh
hdc list targets
```

Install a signed release on a physical device:

```sh
hdc install -r \
  entry/build/default/outputs/default/entry-default-release-signed.hap
hdc shell aa start -b com.richerfu.paws -a EntryAbility
```

For a target selected by key, add `-t <target-key>` immediately after `hdc`.

The smoke script builds, validates the native exports inside the HAP, installs
the app, launches `EntryAbility`, and captures `hilog`:

```sh
scripts/harmony-smoke.sh \
  --hap entry/build/default/outputs/default/entry-default-release-signed.hap \
  --require-protect-success
```

See [HarmonyOS smoke validation](docs/harmony-smoke.md) for profile import,
automatic VPN startup, traffic probes, and troubleshooting options.

## OpenHarmony QEMU

Use a standard-system image with the VPN manager, `VpnExtension`, `/dev/tun`,
policy routing, SettingsData, and the system VPN authorization dialog enabled.
The [ohos-qemu](https://github.com/harmony-contrib/ohos-qemu) project provides
prebuilt images with that capability.

After starting QEMU, connect its forwarded HDC port:

```sh
hdc tconn 127.0.0.1:5555
hdc list targets
```

Then package and run an automated direct-mode VPN smoke:

```sh
scripts/package-hap.sh
scripts/harmony-smoke.sh \
  --no-build \
  --target 127.0.0.1:5555 \
  --hap entry/build/default/outputs/default/entry-default-unsigned.hap \
  --protocol-mode direct \
  --mock-bind 0.0.0.0 \
  --mock-advertise-host 10.0.2.2 \
  --auto-start-vpn \
  --require-protect-success
```

On first use, approve the system VPN authorization dialog. This is QEMU
validation, not the DevEco simulator fallback controlled by
`--allow-vpn-unsupported`.

## Tests

Run all host-side tests:

```sh
cargo test --workspace
```

Generate and validate every local protocol profile:

```sh
scripts/verify-local-protocols.sh
```

Run the complete device/QEMU protocol matrix. Replace the address with one that
the target can use to reach the development host:

```sh
scripts/harmony-protocol-matrix.sh \
  --mock-bind 0.0.0.0 \
  --mock-advertise-host <host-address> \
  --require-protect-success
```

The matrix covers Direct, HTTP, SOCKS5, Shadowsocks, Trojan, and VLESS,
including authentication, unavailable-server, bad-password, and bad-UUID
failure paths. See the [protocol acceptance matrix](docs/app-protocol-acceptance.md).

## Project structure

```text
entry/                         HarmonyOS application module
  src/main/ets/                Entry, backup, and VPN extension abilities
  src/main/cpp/types/          Generated N-API declarations
  src/main/resources/          UI resources and packaged GeoData
crates/
  hmeta_ui/                    Native ArkUI interface and N-API exports
  hmeta_core/                  Runtime lifecycle and meow-rs integration
  hmeta_profile/               Profiles, subscriptions, rules, and persistence
  hmeta_vpn/                   TUN TCP/UDP and DNS forwarding
  hmeta_model/                 Shared data model
local-protocol-tests/          Echo servers, mock proxies, and generated profiles
scripts/                       Build, package, smoke, and regression automation
docs/                          Release, protocol, GeoData, and validation notes
```

## License

[MIT](LICENSE).

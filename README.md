# Paws

Paws is a native HarmonyOS proxy client with an NSH OpenHarmony interface and three
integrated proxy engines: Mihomo, sing-box, and
[meow-rs 0.22.0](https://github.com/meow-rs/meow-rs).
Profiles that use plugin transports are resolved through the route-validated
mihomo engine even when a candidate engine is selected, so a compatibility
preference cannot produce a false Connected state.
Its product flow follows
[Meow for Android](https://github.com/meow-rs/meow-android), while the HarmonyOS UI,
VPN lifecycle, native bridge, packaging, and device validation are implemented
in this repository.

## Architecture

```text
NSH native UI (ui/nsh, local nsh-platform-ohos)
    |  NshSurface / typed commands, domain queries and revision events
    v
Rust application layer
    |  paws_app + paws_core + paws_profile + paws_model
    |  engine policy, profile transactions, session ownership, diagnostics
    |  paws_engine: EngineSession / EngineTunnel / EngineManagement
    |  paws_vpn: TCP/UDP/DNS completion against neutral ports
    v
Backend adapters
    |  paws_engine_meow: meow-rs data plane, listeners and management
    |  paws_engine_go: Mihomo and sing-box lifecycle/configuration/management
    v
Network
```

HarmonyOS abilities own platform permissions, TUN creation and protected network
selection. Every engine follows the same prepare/start/stop/reload bridge.
See [the architecture contract](docs/nsh-three-layer-architecture.md) for
ownership, configuration revisions, disposal and acceptance requirements.

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
- **HarmonyOS integration**: one system continuous-task notification, service
  card, Live View, per-application VPN allow/bypass lists, lifecycle journal,
  and bounded process/exit evidence.
- **Configuration safety**: subscription source, persistent user overrides and
  Harmony runtime adaptation remain separate; preflight reports changed paths
  without exporting credentials.
- **Operations**: target-service health checks, cooled-down automatic failover,
  portable/WebDAV backup, public-exit health, and a redacted diagnostic export.
- **Runtime views**: proxy selection and delay test, live traffic, connections,
  requests, logs, resources, and VPN lifecycle diagnostics.
- **Native UI**: phone, tablet, and 2-in-1 device targets built with the locally
  adapted NSH OpenHarmony backend. Narrow windows use bottom navigation;
  wider windows use a sidebar and bounded content. Configuration drafts stay
  separate from live traffic, resource and session updates. The interface uses
  NSH Gallery's theme and base controls; YAML editing opens a dedicated page
  with NSH's code editor, highlighting, find, undo and revision-checked saving.
- **Localization**: English and Simplified Chinese.
- **Automation**: workspace tests, local protocol mocks, HAP export checks,
  device/QEMU smoke tests, and a 14-mode positive/negative protocol matrix.

## Building

### Prerequisites

- macOS with DevEco Studio and the HarmonyOS SDK installed.
- Rust 1.90 or a compatible newer toolchain.
- [`ohrs`](https://github.com/ohos-rs/ohos-rs) available in `PATH`.
- `hvigorw` and `hdc`; the packaging scripts can use the copies bundled with
  DevEco Studio.
- Signing material configured locally when producing a physical-device HAP.

If DevEco Studio is installed outside `/Applications/DevEco-Studio.app`, set
`DEVECO_STUDIO_HOME` to its `Contents` directory before running these scripts.

This branch uses the locally adapted NSH checkout at `../nsh`. Set `NSH_ROOT`
to select another compatible checkout. `scripts/build-nsh-ohos.sh` builds its
native library and CLI, bundles `ui/nsh`, and stages its ArkTS host plus the
application's capability helpers under this project's `target/ohos`. The
current NSH host attaches through `NshSurface(UIContext)` and uses its native
editor and dialogs without the legacy Ability patch set. Source fingerprints
cover NSH and local binding overrides. Generated outputs stay in Paws.
Set `NSH_ABILITY_ROOT` when the application capability helpers live elsewhere. Run that
script before a standalone `ohpm install --all`; `scripts/package-hap.sh` runs
it automatically.

The project targets HarmonyOS 6.1 and is compatible with HarmonyOS 6.0.2.
The default release package contains `arm64-v8a` native libraries.

### Verify the source tree

```sh
scripts/verify.sh
```

This runs Rust formatting checks, workspace tests, ArkTS platform tests,
local protocol profile generation, and an OpenHarmony `aarch` build.

### Build an unsigned release HAP

Prepare the Go engines from the vendored bridges and pinned upstream sources;
follow [Building Mihomo and sing-box with patched OpenHarmony Go](docs/native-engines-build.md)
for first-time compiler preparation, reusing patched Go 1.24.7, engine patches,
explicit library paths, and release/debug packaging. Keep that guide's exported
environment when running the commands below. A separate FlClash checkout is
no longer required.

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

Keep the private DevEco `build-profile.json5` under `.ohos-sign/`, or set
`PAWS_SIGNING_PROFILE` to its path. The signed packaging command temporarily
applies only its signing configuration and restores the tracked project file,
including when the build fails. Certificates, keys and passwords stay local.

```sh
scripts/fetch-geodata.sh
scripts/package-hap.sh --signed
```

Output:

```text
entry/build/default/outputs/default/entry-default-release-signed.hap
```

Do not unpack and rebuild a signed HAP with a generic ZIP utility. Doing so can
change the aligned resource layout and cause application icon or launcher
problems on physical devices.

See [GeoData provisioning](docs/geodata.md) for rawfile sources and packaging
requirements.

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

The smoke script builds a fresh debug HAP, validates its native exports,
installs the app, launches `EntryAbility`, and captures `hilog`. Pass `--hap`
to reuse an already packaged HAP, such as the signed release:

```sh
scripts/harmony-smoke.sh \
  --hap entry/build/default/outputs/default/entry-default-release-signed.hap \
  --require-protect-success
```

For a route-proven performance sample on a connected phone, use:

```sh
python3 scripts/benchmark-harmony-vpn.py \
  --target <device-ip:hdc-port> \
  --engine mihomo \
  --force-global \
  --requests 1000 \
  --minimum-success-rate 99.5
```

The benchmark fails if it cannot observe the selected proxy in the controller
chain (or independently prove a changed public exit), if the VPN process
restarts, or if the declared success-rate threshold is missed. Public IP values
are stored only as hashes, and the request URL is recorded for reproducibility.

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

Then build a debug HAP and run an automated direct-mode VPN smoke:

```sh
scripts/harmony-smoke.sh \
  --target 127.0.0.1:5555 \
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
failure paths. It builds and packages one debug HAP before running all modes.
Set `HAP_PATH` to reuse an existing debug HAP, or pass `--no-build` to reuse
the default output. Debug Want automation requires `app.debug=true` in the HAP;
for a signed debug package, use
`NATIVE_PROFILE=debug HAP_BUILD_MODE=debug scripts/package-hap.sh --signed`.

## Project structure

```text
entry/                         HarmonyOS application module
  src/main/ets/                Entry, backup, and VPN extension abilities
  src/main/cpp/types/          Generated N-API declarations
  src/main/resources/          UI resources and packaged GeoData
crates/
  paws_ui/                    Thin N-API application/platform binding
  paws_app/                   Commands, preferences and subscription conversion
  paws_engine/                Backend-neutral lifecycle and network contracts
  paws_engine_meow/           meow-rs network adapter
  paws_engine_go/             Mihomo and sing-box adapters
  paws_core/                  Runtime policy, lifecycle and domain orchestration
  paws_profile/               Profiles, subscriptions, rules, and persistence
  paws_vpn/                   TUN TCP/UDP and DNS forwarding
  paws_model/                 Shared data model
ui/nsh/                       NSH native views and state/interaction checks
local-protocol-tests/          Echo servers, mock proxies, and generated profiles
scripts/                       Build, package, smoke, and regression automation
docs/                          Native engine build and GeoData provisioning guides
```

## License

Paws-authored source code uses the [MIT license](LICENSE). Bundled components
retain their own upstream licenses:

- **Mihomo**: [GPL-3.0, full license text](LICENSES/mihomo/LICENSE).
- **sing-box**: [GPL-3.0-or-later and upstream notices](LICENSES/sing-box/LICENSE),
  with the [full GNU GPL v3 text](LICENSES/sing-box/COPYING).

See [third-party notices](THIRD_PARTY_NOTICES.md) and the
[license index](LICENSES/README.md) for pinned source revisions and adaptations.
The Paws MIT grant does not relicense these components or every library in a HAP.

# CI and Release Checks

## Release 1.1.0

Application `versionName` and `buildVersion` are `1.1.0`; `versionCode` is
`1001000`. All Paws workspace crates and the entry/native package manifests
share this version. The About page reads the Cargo package version.

The release artifacts are:

- `dist/release/1.1.0/Paws-1.1.0-release-signed.app`: all three native ABIs,
  signed with the local signing configuration named `release`.
- `dist/release/1.1.0/Paws-1.1.0-arm64-v8a-unsigned.hap`.
- `dist/release/1.1.0/Paws-1.1.0-armeabi-v7a-unsigned.hap`.
- `dist/release/1.1.0/Paws-1.1.0-x86_64-unsigned.hap`.

Compile the native release libraries with:

```sh
scripts/ohrs-build.sh --arch aarch --arch arm --arch x64 --release -p paws_ui
```

Use Arkdown in release mode for final packaging. In an isolated packaging
copy of the project, bind the selected product's `signingConfig` to `release`
for `arkdown build --target app --mode release --no-cache`. For each unsigned
HAP, remove that product's `signingConfig`, include only the selected ABI's
`libpaws_ui.so` and matching NDK `libc++_shared.so`, and set
`buildOption.externalNativeOptions.abiFilters` to that ABI before running
`arkdown build --target hap --mode release --module entry --no-cache`.
This leaves the developer's local signing configuration and library directory
intact.

Verify application versions, release mode, ELF architecture, GeoData rawfiles,
stored icon resources, and the APP's certificate/profile using the SDK signing
tool. Publish SHA-256 checksums alongside the four artifacts. Signing keys and
passwords stay outside source control.

## Local Verification

Run the same checks expected before a pull request:

```sh
scripts/verify.sh
```

The script runs:

- `cargo fmt --check`
- `cargo test --workspace`
- `scripts/verify-local-protocols.sh`
- `scripts/ohrs-build.sh --arch aarch`

For local simulator/device packages that do not need signing, run:

```sh
scripts/package-hap.sh
```

The package script runs `scripts/ohrs-build.sh --arch aarch`, copies the freshly built
`libpaws_ui.so` and the active HarmonyOS NDK's arm64 `libc++_shared.so` into
`entry/libs/arm64-v8a/`, then calls
`hvigorw default@PackageHap --mode module -p module=entry@default`. It
auto-detects DevEco Studio's bundled `hvigorw` and SDK paths when the command
line tools in `PATH` do not match the current HarmonyOS 6.1 project model. It
fails before packaging if the C++ runtime cannot be resolved; `CXX_SHARED_SRC`
can select an explicit runtime from the active SDK. It
passes `--no-daemon` by default so local packaging is not blocked by stale
Hvigor daemon registry locks; set `HVIGOR_ARGS` to override that behavior. The
expected output is
`entry/build/default/outputs/default/entry-default-unsigned.hap`.

For a signed package that can be installed on a physical device, run:

```sh
scripts/package-signed-hap.sh
```

The signed-package script uses DevEco Studio's bundled JBR for the HarmonyOS
signing task. Do not unpack and rebuild the HAP with a generic ZIP utility:
that changes `resources.index` and layered app-icon entries from the aligned,
stored layout emitted by the HarmonyOS packager to deflated entries. Simulator
resource loading may tolerate that layout, while a physical-device launcher
can resolve the desktop icon incorrectly. The script rejects signed artifacts
whose icon resources are compressed. Its default output is
`entry/build/default/outputs/default/entry-default-release-signed.hap`.

## GitHub Actions

`.github/workflows/ci.yml` defines two jobs:

- `rust`: runs on GitHub-hosted macOS and checks Rust formatting, workspace tests, and generated local protocol profiles.
- `harmony`: runs on a self-hosted macOS runner labelled `harmonyos`, builds the aarch debug HAP with `scripts/ohrs-build.sh --arch aarch`, and uploads any generated `.hap` files.

The workspace resolves `meow-*` 0.21.2 from crates.io and pins arkit to a reviewed upstream commit. CI calls `scripts/prepare-ci-cargo.sh` to report that reproducible dependency policy; no machine-local path patches are used.

## Self-Hosted Runner Requirements

The HarmonyOS build runner must provide:

- Rust toolchain compatible with the workspace `rust-version`.
- `ohrs` in `PATH`.
- DevEco/HarmonyOS SDK and native toolchain required by `ohrs`.
- Debug signing materials referenced by `build-profile.json5`, or an equivalent runner-local signing configuration.

If signing material is missing, `scripts/ohrs-build.sh --arch aarch` can fail after Rust compilation succeeds. Keep signing files outside the repository and provision them through runner setup or CI secrets.

Simulator-style local packaging does not require signing; use
`scripts/package-hap.sh` and install the generated unsigned HAP.

## GeoData Rawfiles

Release builds should package real meow-rs geodata rawfiles so GEOIP,
GEOSITE, and ASN rules work on first launch. Run `scripts/fetch-geodata.sh`
before packaging, or provision these files by another trusted release step:

- `entry/src/main/resources/rawfile/geodata/Country.mmdb`
- `entry/src/main/resources/rawfile/geodata/GeoLite2-ASN.mmdb`
- `entry/src/main/resources/rawfile/geodata/geosite.dat`

The binaries are git-ignored. See `docs/geodata.md` for the source URLs and
the exact release provisioning command.

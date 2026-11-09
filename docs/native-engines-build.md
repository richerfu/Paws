# Building Mihomo and sing-box with go-patch

The Go libraries build independently. The current unmodified Meow revision
has an OHOS export-condition error that blocks the complete application HAP;
see the [current upstream build gate](../vendor/README.md).

This guide builds the currently pinned Mihomo and sing-box libraries, then
packages them with Rust/Meow and the native UI in an `arm64-v8a` HAP. Run the
steps in the same shell from the Paws repository root. The bridges and patches
are maintained here; a separate FlClash application checkout is not required.

## 1. Configure the build environment

Install the prerequisites listed in the [README](../README.md#prerequisites),
including DevEco Studio with its OpenHarmony native SDK, Rust and `ohrs`.
The shell scripts also use Git, Python 3, `patch`, `file` and `shasum`.
First-time builds need access to the Git, Go, Cargo and OHPM dependency sources.

```sh
cd /path/to/Paws
export PAWS_ROOT="$PWD"
export DEVECO_STUDIO_HOME=/path/to/DevEco-Studio.app/Contents
export DEVECO_SDK_HOME="$DEVECO_STUDIO_HOME/sdk"
export OHOS_NDK_HOME="$DEVECO_SDK_HOME/default/openharmony"
export GOCACHE="$PAWS_ROOT/target/go-cache"
export GOPATH="$PAWS_ROOT/target/go-work"
export CLASH_META_ROOT="$PAWS_ROOT/target/engine-sources/Clash.Meta"
export SING_BOX_ROOT="$PAWS_ROOT/target/engine-sources/sing-box-ohos"
export PATH="$DEVECO_STUDIO_HOME/tools/node/bin:$DEVECO_STUDIO_HOME/tools/ohpm/bin:$PATH"

test -x "$OHOS_NDK_HOME/native/llvm/bin/clang"
test -x "$OHOS_NDK_HOME/native/llvm/bin/clang++"
test -x "$OHOS_NDK_HOME/native/llvm/bin/aarch64-unknown-linux-ohos-clang"
test -d "$OHOS_NDK_HOME/native/sysroot"
ohpm install --all
```

`DEVECO_STUDIO_HOME` ends in `Contents`; `OHOS_NDK_HOME` ends in
`sdk/default/openharmony`, not `native`. Export both explicitly when DevEco is
installed outside `/Applications`: the standalone engine scripts read
`OHOS_NDK_HOME`, while the HAP packager also reads `DEVECO_STUDIO_HOME`.

## 2. Prepare the pinned engine sources

Clone once into the paths configured above. For existing checkouts, verify the
revisions instead of repeating `git clone`.

```sh
mkdir -p "$PAWS_ROOT/target/engine-sources"
git clone https://github.com/chen08209/Clash.Meta.git "$CLASH_META_ROOT"
git -C "$CLASH_META_ROOT" checkout --detach 8597778c7df410ebe88c57e59eaab32d203156df
git clone --branch v1.13.21 --depth 1 https://github.com/SagerNet/sing-box.git "$SING_BOX_ROOT"
```

| Engine | Pinned source revision | Build entry point |
| --- | --- | --- |
| Mihomo, version `1.10.0` declared by the pinned fork | `8597778c7df410ebe88c57e59eaab32d203156df` | [build-mihomo-ohos.sh](../scripts/build-mihomo-ohos.sh) |
| sing-box `1.13.21` | `628cb31ffa79cffffd34c2f9cde6cae044e4fc12` | [build-sing-box-ohos.sh](../scripts/build-sing-box-ohos.sh) |

```sh
test "$(git -C "$CLASH_META_ROOT" rev-parse HEAD)" = 8597778c7df410ebe88c57e59eaab32d203156df
test "$(git -C "$SING_BOX_ROOT" rev-parse HEAD)" = 628cb31ffa79cffffd34c2f9cde6cae044e4fc12
```

Both build scripts enforce these revisions and apply the engine patches
described below. Each build archives the pinned commit into an owned stage; the selected
checkouts remain unchanged.

## 3. Resolve the pinned go-patch compiler

The build uses the precompiled [go-patch v1.27.3 release](https://github.com/openharmony-command-package/go-patch/releases/tag/v1.27.3).
[scripts/go-ohos-toolchain.json](../scripts/go-ohos-toolchain.json) pins the
release, port identity and SHA256 for each supported macOS/Linux host archive.
Paws does not patch or bootstrap Go sources.

```sh
GO_OHOS_ROOT="$(sh scripts/prepare-go-ohos.sh)"
export GO_OHOS_ROOT
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" version
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" tool dist list | grep -x 'openharmony/arm64'
```

The resolver downloads and verifies the archive, extracts it under
`target/toolchains/go-patch-1.27.3-<host>` and validates `BUILD-INFO.json`, the
compiler version, the OpenHarmony target and the SDK compiler wrappers.
`GO_OHOS_ROOT` may select an existing distribution of the same pinned release;
stock Go and the previous Go 1.24 source distributions are rejected.
Build inventories record both the release identity and the selected Go binary hash.

Both engine scripts use `GOTOOLCHAIN=local`, `GOOS=openharmony`, `GOARCH=arm64`
and `CGO_ENABLED=1`. They invoke go-patch's `cc-arm64`/`cxx-arm64` wrappers with
`OHOS_NDK_HOME` scoped to the SDK's `native` directory; the Paws shell environment
continues to use the parent OpenHarmony SDK directory from step 1.

## 4. Build both native engines

The packager defaults to repository-owned engine libraries. Keep custom
outputs and packaging inputs explicit when overriding them.

```sh
export MIHOMO_CORE_OUTPUT="$PAWS_ROOT/entry/libs/arm64-v8a/libclash.so"
export SING_BOX_OUTPUT="$PAWS_ROOT/entry/libs/arm64-v8a/libsingbox.so"
export MIHOMO_CORE_SRC="$MIHOMO_CORE_OUTPUT"
export MIHOMO_TUN_SRC="$PAWS_ROOT/vendor/mihomo-bridge/tun/tun.go"
export SING_BOX_CORE_SRC="$SING_BOX_OUTPUT"

scripts/build-mihomo-ohos.sh
scripts/build-sing-box-ohos.sh
```

Only Mihomo and sing-box patches are maintained in `patches/`. The build
scripts apply them automatically. NSH, Go and gVisor use their upstream sources.

| Owner | Patch / source | Purpose |
| --- | --- | --- |
| Mihomo checkout | [mihomo-openharmony.patch](../patches/mihomo-openharmony.patch) | OHOS build and DNS adaptation |
| Mihomo checkout | [mihomo-paws-management.patch](../patches/mihomo-paws-management.patch) | Session-owned management, provider retirement and public-service integration |
| sing-box checkout | [sing-box-http-obfs-handshake.patch](../patches/sing-box-http-obfs-handshake.patch) and [sing-box-http-obfs-first-flight.patch](../patches/sing-box-http-obfs-first-flight.patch) | HTTP-obfs handshake and first-flight behavior |
| sing-box checkout | [sing-box-paws-management.patch](../patches/sing-box-paws-management.patch) | Managed runtime, providers, DNS policy and authenticated management integration |
| Both bridges | [vendor/mihomo-bridge](../vendor/mihomo-bridge/) and [vendor/sing-box-bridge](../vendor/sing-box-bridge/) | Paws entry points, platform integration and telemetry |

Mihomo rebuilds `target/mihomo-ohos-build` on each invocation. It stages the
bridge and resolves the locked gVisor dependency without a local source override. The Go build uses `-tags=with_gvisor -buildmode=c-shared`.
Rust loads its C ABI directly; the old Mihomo N-API wrapper is not built or
packaged by this path.

sing-box archives its pinned source into an owned build stage, applies the
declared patches and copies the bridge into staged `cmd/libtinypaws`, then builds
with `-buildmode=c-shared` and
`with_gvisor,with_quic,with_dhcp,with_utls,with_clash_api`.
Both scripts use go-patch's wrappers to select the matching SDK target and sysroot. Source inventories are compared before publishing either
library; neither script patches the selected source checkout or module cache.

| Output under `entry/libs/arm64-v8a/` | Role |
| --- | --- |
| `libclash.so` | Mihomo Go engine |
| `libsingbox.so` | sing-box Go engine; generated C declarations remain build outputs |
| `libpaws_ui.so` | Thin Rust N-API application/platform binding; links the application, Core and engine adapters |
| `libnsh_platform_ohos.so` | NSH native view runtime; production views are authored in `ui/nsh` |
| `libnative_ability.so`, `libc++_shared.so` | Matching native Ability and C++ runtime dependencies |

The sing-box bridge exports `TinyPawsSingTraffic` for TCP/UDP session totals and
elapsed-time rates. Rust engine adapters own C ABI loading and normalize traffic;
the abilities consume the backend-neutral application binding. The obsolete
Mihomo `libcore.so` N-API shim is not a production dependency. Rebuild native
engines, the Rust application binding and the matching ArkTS declarations as a
coherent candidate; packaging must verify the actual native libraries and NSH
UI bundle rather than relying on their filenames.

## 5. Package the current libraries into a HAP

Keep the environment from steps 1–4 in the shell. Provision the raw GeoData
files on the first build; see [geodata.md](geodata.md) for details.

```sh
scripts/fetch-geodata.sh
```

Choose one package variant:

```sh
# Unsigned release, the default.
NATIVE_PROFILE=release HAP_BUILD_MODE=release scripts/package-hap.sh
```

```sh
# Signed release; requires a valid local Hvigor signing configuration.
NATIVE_PROFILE=release HAP_BUILD_MODE=release scripts/package-hap.sh --signed
```

```sh
# Signed debug build for device debugging; also requires signing configuration.
NATIVE_PROFILE=debug HAP_BUILD_MODE=debug scripts/package-hap.sh --signed
```

| Variant | Output under `entry/build/default/outputs/default/` |
| --- | --- |
| Unsigned release | `entry-default-unsigned.hap` |
| Signed release | `entry-default-release-signed.hap` |
| Signed debug | `entry-default-debug-signed.hap` |

`package-hap.sh` rebuilds repository-owned Go outputs when they are missing
or bridge/patch inputs are newer, builds the Rust library with
`scripts/ohrs-build.sh`, refreshes the generated N-API declarations and invokes
Hvigor. After changing the compiler, SDK, build flags or an upstream checkout,
explicitly rerun both engine build scripts: timestamp checks alone do not
cover all of those inputs.

`dist/Paws-build-manifest.json` records the build fingerprint, HAP hash and
the comparison of the packaged native ELF load segments against the compiled
libraries. Hvigor may strip non-loaded sections, so whole-file `.so` hashes
can differ even when the loaded image matches.

## 6. Verify the outputs

These commands inspect the generated libraries without installing the app:

```sh
file "$MIHOMO_CORE_OUTPUT" "$SING_BOX_OUTPUT"
shasum -a 256 "$MIHOMO_CORE_OUTPUT" "$SING_BOX_OUTPUT"
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" version -m "$MIHOMO_CORE_OUTPUT"
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" version -m "$SING_BOX_OUTPUT"
"$OHOS_NDK_HOME/native/llvm/bin/llvm-nm" -D --defined-only "$SING_BOX_OUTPUT" | grep 'TinyPawsSingTraffic'
```

Expect ARM64 ELF shared objects and Go build metadata containing
`GOOS=openharmony`, `GOARCH=arm64`, and `CGO_ENABLED=1`.

Run host-side bridge tests with a host Go compiler in `PATH`. For the traffic
tests, `SING_BOX_ROOT` must still point to the pinned checkout:

```sh
sh scripts/test-sing-box-traffic.sh
```

On the macOS environment used for the traffic fix, host Go 1.24.7 encountered
linker/runtime failures before the tests ran. The same tests passed with:

```sh
GOTOOLCHAIN=go1.26.5 sh scripts/test-sing-box-traffic.sh
```

That override may download a host toolchain. It is only for host tests;
engine compilation continues to use the pinned go-patch release. The broader
repository checks are available through `scripts/verify.sh`.
Compilation and package checks do not replace device verification: after
installing a signed HAP, test both engines with active upload/download traffic,
idle periods, disconnect and reconnect, and inspect the speed chart.

## Troubleshooting

| Symptom | Check / recovery |
| --- | --- |
| `exec format error` when running `bin/go` | Use `prepare-go-ohos.sh` to resolve the archive for the current host. |
| `unsupported GOOS/GOARCH pair openharmony/arm64` or unwanted toolchain download | Verify `GO_OHOS_ROOT/bin/go tool dist list`; build through the scripts, which use that compiler with `GOTOOLCHAIN=local`. |
| Source revision mismatch or patch rejects | Check the pinned commit and local diff. Use a fresh dedicated checkout if the existing one contains unrelated changes; do not force the patch onto another revision. |
| Clang, sysroot or `libc++_shared.so` unavailable | Check `OHOS_NDK_HOME` against the installed native SDK and use the same DevEco installation for packaging. |
| Module download fails | Check network/proxy access for the selected `GOPATH`; keep the compiler pinned while resolving dependency access. |
| Missing `TinyPawsSingTraffic`, or old `updateSingBoxVpnTraffic` declarations | Rebuild sing-box and run `package-hap.sh` to rebuild Rust and refresh ArkTS declarations together. |

# Building Mihomo and sing-box with patched OpenHarmony Go

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
described below. Modified and generated files in these dedicated checkouts
are expected after building.

## 3. Prepare or reuse patched Go

The engine compiler must run on the build machine, report `go1.24.7`, and
support the `openharmony/arm64` target. A stock Go installation is useful for
bootstrapping and host tests; the engine builds use the patched OHOS compiler
by absolute path.

### First build from the OHOS Go distribution

Extract the OpenHarmony SIG `go1.24.5.ohosv1r1` distribution and locate the
directory containing `VERSION` and `src/make.bash`. The distributed Linux
x86-64 executable cannot run on an Apple Silicon Mac, but its source can be
used to build a native compiler.

Use a working host Go installation **at least 1.22.6** as the bootstrap
compiler. `GOROOT_BOOTSTRAP` is its installation root, containing `bin/go`.
For example, an existing Homebrew Go 1.22.5 is too old for this step.

```sh
export GO_OHOS_SOURCE="$HOME/Downloads/go1.24.5.ohosv1r1"
export GOROOT_BOOTSTRAP=/path/to/host-go
test -f "$GO_OHOS_SOURCE/src/make.bash"
head -1 "$GO_OHOS_SOURCE/VERSION"
GOTOOLCHAIN=local "$GOROOT_BOOTSTRAP/bin/go" version

# Build logs go to stderr; stdout is the resulting compiler directory.
GO_OHOS_ROOT="$(sh scripts/prepare-go-ohos.sh "$GO_OHOS_SOURCE")"
export GO_OHOS_ROOT
```

[prepare-go-ohos.sh](../scripts/prepare-go-ohos.sh) copies the distribution to
`target/toolchains/go1.24.7-ohos-<host>`, applies
[go-ohos-1.24.7-compat.patch](../patches/go-ohos-1.24.7-compat.patch) with
`patch -p1 -F 0`, then runs `src/make.bash`. On Apple Silicon the default
directory is `target/toolchains/go1.24.7-ohos-darwin-arm64`.
The original distribution remains unchanged.

The patch contains the remaining changes missing from the OHOS distribution
between these upstream Go revisions:

| Base | Revision |
| --- | --- |
| Go 1.24.5 | `9d828e80fa1f3cc52de60428cae446b35b576de8` |
| Go 1.24.7 | `d41a8222133aabaf5527eab64586050a0fbb773b` |

It updates six files, including version metadata, toolchain prerelease
handling and IPv4-mapped UDP address handling, while retaining the OHOS
runtime/syscall changes. Apply it to the OHOS distribution through the helper;
it is not a complete OHOS port for a stock Go source tree. The Go license and
source attribution are recorded in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

### Reuse an already prepared compiler

On subsequent builds, skip bootstrapping and point to the existing compiler.
For the default Apple Silicon layout:

```sh
export GO_OHOS_ROOT="$PAWS_ROOT/target/toolchains/go1.24.7-ohos-darwin-arm64"
```

Both engine scripts call the helper internally. An executable compiler at
`GO_OHOS_ROOT` that reports exactly `go1.24.7` and supports
`openharmony/arm64` is reused without rebuilding. `GO_OHOS_ROOT` can also point
to the original distribution, in which case the helper resolves its cache.

Check the selected compiler before building:

```sh
test -x "$GO_OHOS_ROOT/bin/go"
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" version
GOTOOLCHAIN=local "$GO_OHOS_ROOT/bin/go" tool dist list | grep -x 'openharmony/arm64'
```

For Apple Silicon the expected version line is
`go version go1.24.7 darwin/arm64`; the second command must print
`openharmony/arm64`. Both engine scripts set `GOTOOLCHAIN=local`,
`GOOS=openharmony`, `GOARCH=arm64` and `CGO_ENABLED=1` for compilation.
Keep those settings scoped to the engine commands so host tests still build
for macOS.

If the Go patch or original distribution changes, prepare a new cache from
the source distribution:

```sh
# Select a directory that does not already exist.
export GO_OHOS_HOST_ROOT="$PAWS_ROOT/target/toolchains/go1.24.7-ohos-rebuild-01"
GO_OHOS_ROOT="$(sh scripts/prepare-go-ohos.sh "$GO_OHOS_SOURCE")"
export GO_OHOS_ROOT
```

The cache signature records the source path and patch hash. Direct reuse of an
already executable `GO_OHOS_ROOT` checks its version and target support, so it
does not by itself apply a subsequently edited Go patch.

## 4. Build both native engines

Keep the build outputs and packaging inputs explicit. This also prevents
`package-hap.sh` from selecting libraries from a legacy sibling
`FlClash-ohos-upstream` checkout.

```sh
export MIHOMO_CORE_OUTPUT="$PAWS_ROOT/entry/libs/arm64-v8a/libclash.so"
export MIHOMO_WRAPPER_OUTPUT="$PAWS_ROOT/entry/libs/arm64-v8a/libcore.so"
export SING_BOX_OUTPUT="$PAWS_ROOT/entry/libs/arm64-v8a/libsingbox.so"
export MIHOMO_CORE_SRC="$MIHOMO_CORE_OUTPUT"
export MIHOMO_WRAPPER_SRC="$MIHOMO_WRAPPER_OUTPUT"
export MIHOMO_TUN_SRC="$PAWS_ROOT/vendor/mihomo-bridge/tun/tun.go"
export SING_BOX_CORE_SRC="$SING_BOX_OUTPUT"

scripts/build-mihomo-ohos.sh
scripts/build-sing-box-ohos.sh
```

The Go compatibility patch above and the engine patches below are different
layers. The build scripts apply the engine patches automatically.

| Owner | Patch / source | Purpose |
| --- | --- | --- |
| Mihomo checkout | [mihomo-openharmony.patch](../patches/mihomo-openharmony.patch) | OHOS build and DNS adaptation |
| Mihomo staged bridge | [mihomo-ohos-tun-fd-ownership.patch](../patches/mihomo-ohos-tun-fd-ownership.patch) | Preserve HarmonyOS ownership of the TUN descriptor; applied if the copied bridge still closes it |
| Mihomo staged gVisor | [gvisor-fstat.patch](../patches/gvisor-fstat.patch) | Adapt `IsSocketFD` for the pinned gVisor module |
| sing-box checkout | [sing-box-http-obfs-handshake.patch](../patches/sing-box-http-obfs-handshake.patch) and [sing-box-http-obfs-first-flight.patch](../patches/sing-box-http-obfs-first-flight.patch) | HTTP-obfs handshake and first-flight behavior |
| sing-box checkout | [sing-box-benchmark-probe.patch](../patches/sing-box-benchmark-probe.patch) | Local protocol probe support |
| Both bridges | [vendor/mihomo-bridge](../vendor/mihomo-bridge/) and [vendor/sing-box-bridge](../vendor/sing-box-bridge/) | Paws entry points, platform integration and telemetry |

Mihomo rebuilds `target/mihomo-ohos-build` on each invocation. It stages the
bridge and `github.com/metacubex/gvisor@v0.0.0-20260922041103-e2cbcd6e7400`,
then patches that copy with `patch -p1 -F 0`. It leaves the shared Go module
cache unchanged. The Go build uses `-tags=with_gvisor -buildmode=c-shared`,
and the SDK's `clang++` compiles the N-API wrapper against `libclash.so`.
Treat `libclash.so` and `libcore.so` as a matching pair.

sing-box copies the bridge into its checkout's `cmd/libtinypaws`, then builds
with `-buildmode=c-shared` and
`with_gvisor,with_quic,with_dhcp,with_utls,with_clash_api`.
Both scripts supply the SDK compiler; Mihomo also passes the OHOS target and
sysroot explicitly.

| Output under `entry/libs/arm64-v8a/` | Role |
| --- | --- |
| `libclash.so` | Mihomo Go engine |
| `libcore.so` | Mihomo ArkTS/N-API wrapper |
| `libsingbox.so`, `libsingbox.h` | sing-box Go engine and generated C declarations |
| `libpaws_ui.so` | Rust UI, Meow and sing-box FFI; built by the HAP packager |

The current sing-box bridge includes `traffic.go` and exports
`TinyPawsSingTraffic` for TCP/UDP session totals and elapsed-time rates.
Rebuild `libsingbox.so` together with `libpaws_ui.so` and the ArkTS layer for
the traffic-chart fix. The packager regenerates and installs the N-API type
declarations, including `updateSingBoxVpnTraffic`.

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
file "$MIHOMO_CORE_OUTPUT" "$MIHOMO_WRAPPER_OUTPUT" "$SING_BOX_OUTPUT"
shasum -a 256 "$MIHOMO_CORE_OUTPUT" "$MIHOMO_WRAPPER_OUTPUT" "$SING_BOX_OUTPUT"
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
engine compilation continues to use patched OHOS Go 1.24.7. The broader
repository checks are available through `scripts/verify.sh`.
Compilation and package checks do not replace device verification: after
installing a signed HAP, test both engines with active upload/download traffic,
idle periods, disconnect and reconnect, and inspect the speed chart.

## Troubleshooting

| Symptom | Check / recovery |
| --- | --- |
| `exec format error` when running the distribution's `bin/go` | Use its source with `prepare-go-ohos.sh` to build a compiler for the host machine. |
| Bootstrap Go is missing or too old | Set `GOROOT_BOOTSTRAP` to a host installation root with Go >= 1.22.6; verify its `bin/go version`. |
| `Compiler cache belongs to a different source` | Select a new `GO_OHOS_HOST_ROOT` and prepare from the original distribution; retain the old cache until the new compiler is verified. |
| `unsupported GOOS/GOARCH pair openharmony/arm64` or unwanted toolchain download | Verify `GO_OHOS_ROOT/bin/go tool dist list`; build through the scripts, which use that compiler with `GOTOOLCHAIN=local`. |
| Source revision mismatch or patch rejects | Check the pinned commit and local diff. Use a fresh dedicated checkout if the existing one contains unrelated changes; do not force the patch onto another revision. |
| Clang, sysroot or `libc++_shared.so` unavailable | Check `OHOS_NDK_HOME` against the installed native SDK and use the same DevEco installation for packaging. |
| Module download fails | Check network/proxy access for the selected `GOPATH`; keep the compiler pinned while resolving dependency access. |
| Package contains an old Mihomo library | Set the explicit `MIHOMO_*_SRC` paths from step 4 so an old sibling FlClash checkout cannot override the local build. |
| Missing `TinyPawsSingTraffic`, or old `updateSingBoxVpnTraffic` declarations | Rebuild sing-box and run `package-hap.sh` to rebuild Rust and refresh ArkTS declarations together. |

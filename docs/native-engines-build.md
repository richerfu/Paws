# Building the native engines

TinyPaws packages Rust/Meow, Mihomo and sing-box in an arm64 HAP. The Go bridges
and N-API wrapper are in `vendor/`; the two upstream engine checkouts are pinned
by the build scripts. A FlClash application checkout or its precompiled libraries
are not required.

## Source checkouts

Run from the TinyPaws repository root:

```sh
mkdir -p target/engine-sources
git clone https://github.com/chen08209/Clash.Meta.git target/engine-sources/Clash.Meta
git -C target/engine-sources/Clash.Meta checkout --detach 8597778c7df410ebe88c57e59eaab32d203156df
git clone --branch v1.13.21 --depth 1 https://github.com/SagerNet/sing-box.git target/engine-sources/sing-box-ohos
export CLASH_META_ROOT="$PWD/target/engine-sources/Clash.Meta"
export SING_BOX_ROOT="$PWD/target/engine-sources/sing-box-ohos"
```

The expected sing-box revision is
`628cb31ffa79cffffd34c2f9cde6cae044e4fc12`.
Both scripts apply the repository's engine patches to these working copies.
Mihomo's gVisor module is copied into an isolated staging directory and patched
with `patch -p1 -F 0`; it does not change the shared Go module cache. This avoids
`git apply` silently skipping files inside an untracked directory of the parent
TinyPaws repository. The gVisor patch targets `IsSocketFD` at the pinned module
revision, and the vendored TUN bridge retains HarmonyOS's ownership of the fd.

## OHOS Go compiler

The `go1.24.5.ohosv1r1` distribution contains OpenHarmony support, but its bundled
Linux x86-64 compiler cannot execute on an Apple Silicon Mac. sing-box 1.13.21
also requires Go 1.24.7. Use the distribution's source to build a host compiler:

```sh
export GO_OHOS_ROOT=/Users/ranger/Downloads/go1.24.5.ohosv1r1
# Set this to a native host Go installation >= 1.22.6 on a fresh machine.
export GOROOT_BOOTSTRAP=/path/to/native/go
scripts/prepare-go-ohos.sh "$GO_OHOS_ROOT"
```

The helper copies the source to `target/toolchains/go1.24.7-ohos-<host>`, applies
`patches/go-ohos-1.24.7-compat.patch`, and runs Go's `src/make.bash`. The input
distribution is left untouched. An existing native OHOS Go 1.24.7 installation
can instead be passed directly as `GO_OHOS_ROOT`. Engine build scripts invoke
the helper automatically, and use `GOTOOLCHAIN=local` to retain OHOS support.
`GO_OHOS_HOST_ROOT` can select another empty compiler cache directory.

The compatibility patch was derived from the official Go source tags:

- Go 1.24.5: `9d828e80fa1f3cc52de60428cae446b35b576de8`.
- Go 1.24.7: `d41a8222133aabaf5527eab64586050a0fbb773b`.
- Upstream: <https://github.com/golang/go> (BSD-style license).

The OHOS source distribution already contains most changes between these
tags. The patch supplies the remaining changes in six files, including the
IPv4-mapped UDP address fix, toolchain prerelease handling and the version
metadata. It preserves the existing OpenHarmony runtime/syscall changes and
does not lower sing-box's Go version requirement.

## Build and package

Install Rust/ohrs and the DevEco Studio SDK, then configure the local paths:

```sh
export DEVECO_STUDIO_HOME=/path/to/DevEco-Studio.app/Contents
export OHOS_NDK_HOME="$DEVECO_STUDIO_HOME/sdk/default/openharmony"
export GOCACHE="$PWD/target/go-cache"
export GOPATH="$PWD/target/go-work"
export PATH="$DEVECO_STUDIO_HOME/tools/node/bin:$DEVECO_STUDIO_HOME/tools/ohpm/bin:$PATH"
ohpm install --all
scripts/build-mihomo-ohos.sh
scripts/build-sing-box-ohos.sh
# Required once if GeoData files are absent:
scripts/fetch-geodata.sh
scripts/package-hap.sh
```

The engine builds produce `entry/libs/arm64-v8a/libclash.so`, `libcore.so` and
`libsingbox.so`. Packaging rebuilds repository-owned Go outputs when bridge or
patch inputs are newer, builds the Rust release library and assembles
`entry/build/default/outputs/default/entry-default-unsigned.hap`.
`dist/TinyPaws-build-manifest.json` records the build fingerprint, HAP hash and
comparison of each packaged ELF's loaded segments with its compiled library.

Signing requires a configured local HarmonyOS profile; use
`scripts/package-hap.sh --signed` after configuring it. Compilation and package
verification do not establish real-device VPN behavior or protocol acceptance.

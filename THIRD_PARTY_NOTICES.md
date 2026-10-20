# Third-party components

Paws can package an optional mihomo compatibility engine for HarmonyOS.

- FlClash OpenHarmony core bridge (`vendor/mihomo-bridge` and `vendor/mihomo-bridge/ohos-napi`): GPL-3.0.
- mihomo (`https://github.com/chen08209/Clash.Meta.git`, commit `8597778c7df410ebe88c57e59eaab32d203156df` with `patches/mihomo-openharmony.patch` and `patches/gvisor-fstat.patch`): GPL-3.0.
- The packaged `libcore.so` and `libclash.so` are compiled via `scripts/build-mihomo-ohos.sh` from `vendor/mihomo-bridge` and commit `8597778c` (2026-10-01).

The corresponding source and license texts are included in the Paws source
delivery. When redistributing a HAP containing these libraries, distribute the
matching source and GPL-3.0 notices with it.

Paws also packages an independent sing-box engine for comparative and
advanced-protocol use.

- sing-box 1.13.21, commit `628cb31f`, GPL-3.0-or-later.
- OpenHarmony Go bridge source: `vendor/sing-box-bridge/main.go`.
- The packaged `libsingbox.so` is compiled for `openharmony/arm64` with the
  OpenHarmony SIG Go 1.24 toolchain and the DevEco OpenHarmony clang/sysroot.

The Go compiler can be rebuilt from the OpenHarmony SIG Go 1.24.5 source
distribution with `patches/go-ohos-1.24.7-compat.patch`. This patch contains the
remaining official Go 1.24.5-to-1.24.7 changes missing from that distribution;
the existing OpenHarmony changes are retained. Go uses a BSD-style license.
See `docs/native-engines-build.md` for source revisions and build commands.

Redistribution of a HAP containing `libsingbox.so` must include the matching
sing-box source, bridge source, and GPL-3.0-or-later license notice.

The Rust native engine uses meow-rs 0.22.0. The local copies of `meow-common`,
`meow-proxy`, `meow-anytls`, `meow-listener`, `meow-tunnel`, and `meow-transport`
in `vendor/` retain their original source and license notices. Paws changes
add OpenHarmony socket protection, network-bound bootstrap DNS, HTTP-obfs
compatibility and bounded diagnostics; see `vendor/TINYPAWS-PATCHES.md`.

The sing-box bridge additionally includes `physical_network.go`, `obfs_plugin.go`,
`trace.go`, and `httpobfs/conn.go`; the required upstream handshake change is
`patches/sing-box-http-obfs-handshake.patch`. Rebuilding requires applying that
patch, as enforced by `scripts/build-sing-box-ohos.sh`.

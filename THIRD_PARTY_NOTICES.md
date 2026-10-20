# Third-party components

TinyPaws can package an optional mihomo compatibility engine for HarmonyOS.

- FlClash OpenHarmony core bridge (`vendor/mihomo-bridge` and `vendor/mihomo-bridge/ohos-napi`): GPL-3.0.
- mihomo (`https://github.com/chen08209/Clash.Meta.git`, commit `8597778c7df410ebe88c57e59eaab32d203156df` with `patches/mihomo-openharmony.patch` and `patches/gvisor-fstat.patch`): GPL-3.0.
- The packaged `libcore.so` and `libclash.so` are compiled via `scripts/build-mihomo-ohos.sh` from `vendor/mihomo-bridge` and commit `8597778c` (2026-10-01).

The corresponding source and license texts are included in the TinyPaws source
delivery. When redistributing a HAP containing these libraries, distribute the
matching source and GPL-3.0 notices with it.

TinyPaws also packages an independent sing-box engine for comparative and
advanced-protocol use.

- sing-box 1.13.21, commit `628cb31f`, GPL-3.0-or-later.
- OpenHarmony Go bridge source: `vendor/sing-box-bridge/main.go`.
- The packaged `libsingbox.so` is compiled for `openharmony/arm64` with the
  OpenHarmony SIG Go 1.24 toolchain and the DevEco OpenHarmony clang/sysroot.

Redistribution of a HAP containing `libsingbox.so` must include the matching
sing-box source, bridge source, and GPL-3.0-or-later license notice.

The Rust native engine uses meow-rs 0.21.2. The local copies of `meow-common`,
`meow-proxy`, and `meow-anytls` in `vendor/` retain their original source and
license notices. TinyPaws changes add OpenHarmony socket protection and
network-bound bootstrap DNS; those changes are distributed with this source.

The sing-box bridge additionally includes `physical_network.go`, `obfs_plugin.go`,
`trace.go`, and `httpobfs/conn.go`; the required upstream handshake change is
`patches/sing-box-http-obfs-handshake.patch`. Rebuilding requires applying that
patch, as enforced by `scripts/build-sing-box-ohos.sh`.

# TinyPaws 1.2.7 delivery

Date: 2026-10-15

## Release artifact

- Package: `dist/TinyPaws-1.2.7-ohos-arm64-signed.hap`
- Bundle: `com.richerfu.paws`
- Version: `1.2.7-tiny` / versionCode `1002007`
- SHA-256: `79242ca491845918bc68c12a51a83cff6a4e5743f3484ca0b2522a5cca03d6c8`
- Installation: signed, profile-verified and installed in place on the physical
  HarmonyOS device. Existing subscription, selections and app identity were
  preserved; the final stored engine preference is Mihomo.

## Stability hotfix

- The 2026-10-15 09:01 drop was a clean application-driven shutdown, not a
  crash or low-memory kill. The VPN had run for 13 minutes 6 seconds when
  cleanup began. That timing and the source path point to the old auxiliary
  `dataTransfer` cancellation callback, which called `requestStopVpn()`; the
  platform reason code had already rotated out of hilog, so this cause remains
  an evidence-backed inference. The task is now treated only as scheduler
  protection and notification state; its cancellation is recorded while the
  established VpnExtension/TUN session remains active.
- The 2026-10-14 19:08 VPN-process crash was a `SIGBUS(BUS_OBJERR)` in
  `libclash.so`. The fault address mapped to the live `Country.mmdb`. TinyPaws
  had copied refreshed geodata over Mihomo's memory-mapped file, truncating the
  backing inode. Geodata is now copied to a fresh temporary inode and renamed
  atomically, and Mihomo's independent geodata updater is disabled.
- Three earlier VPN-process failures were HarmonyOS FDSAN aborts in
  `NetworkVpnClient::DestroyVpn`: the Mihomo Harmony bridge closed the original
  system-owned TUN descriptor after duplicating it. The bridge now borrows the
  original descriptor and transfers ownership only for its duplicate.
- The packaging gate rejects an unpatched or stale external Mihomo build. The
  required source change is retained as
  `patches/mihomo-ohos-tun-fd-ownership.patch`.
- Physical-device acceptance covered a live Mihomo configuration reload and
  three complete VPN stop/start cycles. The VPN stayed connected after reload,
  each stop removed the VPN process cleanly, every restart created a new TUN,
  and HiviewFaultLogger gained no new `cppcrash` or `fdsan` entry.
- The continuous-task fix then ran in the background from 09:14:14 through
  09:30:00 (15 minutes 46 seconds), crossing the previous 13-minute-6-second
  failure window. The UI/VPN PIDs stayed 44103/44305, `vpn-tun` remained
  `UP/RUNNING`, packet counters advanced from 627/618 to 958/949, VPN RSS stayed
  near 140 MiB, and no cleanup, system-exit, `cppcrash`, or FDSAN event appeared.

## Delivered scope

- Source history: release commit and annotated `TinyPaws-1.2.7` tag.
- Engine choices: native Rust Meow, packaged Mihomo, and sing-box bridges.
  Plugin profiles use a session-level capability fallback to Mihomo until the
  candidate route passes the same physical-device gate.
- Background lifecycle: one HarmonyOS `dataTransfer` continuous task and one
  system-owned notification path, decoupled continuous-task and VPN ownership,
  lifecycle/OS-exit journal, default-network monitor with debounced reload/reset,
  and exact process ownership recovery.
- Networking: generic DNS-to-flow hostname recovery, target health/failover,
  per-app trusted/bypass lists, user rule/DNS overrides, and configuration
  preflight.
- Product integration: redacted diagnostics, portable/WebDAV backup, service
  card and Live View.

## Verification

| Gate | Result |
| --- | --- |
| Full workspace verification | Passed: Rust core/model/profile/UI, ArkTS 43/43, 15 local protocol profiles, OHOS builds |
| Continuous-task regression window | Passed: 15m46s background run; same UI/VPN PIDs; TUN active; no new exit/fault |
| Meow preference on plugin profile | 50/50 through capability fallback; route proven; stable PID |
| sing-box preference on plugin profile | 50/50 through capability fallback; route proven; stable PID |
| 1,000 request benchmark | 1,000/1,000; median 544 ms; p95 1,206 ms; p99 1,535 ms; stable PID |
| 10,000 request background benchmark | 9,959/10,000 (99.59%); median 524 ms; p95 890 ms; p99 1,394 ms; 41 timeouts; stable PID |
| Raw Meow on current plugin provider | Failed 0/10 with peer resets; retained as negative evidence |
| Raw sing-box on current plugin provider | Failed 0/10 with peer resets; retained as negative evidence |

The performance reports contain hashed route identifiers and exit values. They
do not contain the subscription URL, credentials, public IP address or node
name.

## Acceptance boundary

The implementation and bounded physical-device gates are complete. Three
eight-hour lock-screen runs, repeated Wi-Fi/mobile handovers and a 24-hour
memory observation remain time-based field acceptance. The included lifecycle,
process and route evidence makes those runs diagnosable; this release does not
claim they happened inside the bounded delivery session.

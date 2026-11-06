# Changelog

## Unreleased

- Restore the Paws product name and 1.2.0 application version metadata.
- Reintegrate multi-engine settings, connection diagnostics, automatic recovery,
  and local/WebDAV backup into the 1.2.0 navigation and shadcn components.
  Keep text fields click-focused during scrolling and give settings entries,
  dividers, helper text, and actions consistent spacing.
- Report the actual VPN session's engine and startup checks instead of showing
  the configured preference or cached checks from another session.
- Complete the VPN start receipt as soon as the tunnel is connected; run HTTPS
  health checks in the background with session-scoped fallback. Handle Stop
  events before Go-engine configuration reloads, and retain the control
  subscription after a configuration error.
- Show Mihomo, sing-box, and meow-rs versions and open-source links in About.
  Include the pinned Mihomo and sing-box license notices and complete GPL v3
  text in the repository, with explicit third-party license references.
- Keep automatic engine selection on the verified Mihomo production baseline.
  Profiles with unaudited proxy providers also retain candidate fallback instead
  of treating a missing inline plugin as evidence of compatibility.
- Keep an established VPN alive when HarmonyOS cancels the auxiliary
  `dataTransfer` continuous task for low traffic or stale progress. The task
  protects active transfers but no longer owns the VPN lifetime.
- Replace packaged Mihomo geodata atomically and disable its independent
  geodata updater so a live memory-mapped `Country.mmdb` cannot be truncated
  during configuration reload.
- Keep the HarmonyOS-owned TUN descriptor open in the Mihomo bridge and close
  only its duplicate, preventing FDSAN aborts during VPN teardown.
- Allow profile export as a QR code with PNG saving. Subscription profiles share
  their URL; local profiles share YAML when it fits in one code, and scanned
  YAML profiles can be imported.

## 1.2.7-tiny — 2026-10-14

- Omit absent HarmonyOS application-routing arrays instead of sending
  `undefined` values through the VPN binder; ANCO/出境易 containers continue
  to use the documented device-wide compatibility route.
- Align the Rust simple-obfs HTTP handshake header order and wire casing with
  mihomo for strict deployed frontends, while retaining automatic mihomo
  selection for plugin profiles until the alternative engines pass the same
  device route gate.
- Apply that capability gate to explicit Meow and sing-box preferences as well,
  preventing unsupported plugin paths from reporting a false Connected state.
- Monitor default-network changes while connected and reload or reset the
  active data plane after a debounced handover.
- Make the full verification script discover the installed DevEco SDK and add
  progress plus a declared 99.5% acceptance threshold to 1,000/10,000 request
  device benchmarks.

## 1.2.6-tiny — 2026-10-14

- Rename the installed product to TinyPaws while preserving the existing
  `com.richerfu.paws` data and signing identity for in-place upgrades.
- Add generic DNS-to-flow hostname recovery, including fragmented TLS, and fix
  app-managed DNS listener/self-reference conflicts.
- Add configuration preflight, target-service health/failover, public-exit
  status, redacted diagnostics, process/OS-exit evidence and quantified memory
  trends.
- Add HarmonyOS per-application VPN policy, one system continuous-task
  notification, Live View, service card, portable backup and WebDAV backup.
- Package mihomo and sing-box bridges while retaining mihomo as the automatic
  choice for plugin transports that have passed physical-device validation.
- Add a route-proven device benchmark which checks controller chain, request
  reliability, latency and VPN process identity without exporting public IPs.

## 1.1.0 — 2026-09-16

- Keep dashboard status focused on the VPN lifecycle; configuration reload and
  other operation errors no longer replace the VPN status.
- Detect an unexpectedly terminated VPN process through its released ownership
  lease and refresh the dashboard. Harden session ownership, recovery, and
  cross-process state notifications over ashmem and Unix sockets.
- Add QR code subscription imports, subscription conversion, custom rule
  imports, rule lookup, and quick rule creation.
- Improve proxy group semantics, routing mode selection, profile refresh,
  cancellable imports, and global proxy selection.
- Add opt-in persistent daily logs and improve network controls and privacy
  information.
- Refresh the native UI, improve first-frame layout and resource handling, and
  upgrade Arkit and meow-rs (0.21.2).
- Remove redundant VPN lifecycle logs and export release native libraries for
  arm64-v8a, armeabi-v7a, and x86_64.
- Align application metadata, native package versions, and the About page with
  version 1.1.0 (versionCode 1001000).

## 1.0.0 — 2026-07-26

- Initial tagged release of the native HarmonyOS Clash/mihomo client.

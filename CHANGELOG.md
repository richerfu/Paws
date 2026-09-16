# Changelog

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

# TinyPaws engine evaluation — 2026-10-14

This report distinguishes configuration compatibility, HarmonyOS socket routing,
transport compatibility and measured reliability. Every candidate is tested
through a listener owned by that engine. A successful build, a running process,
or a request sent to another engine is not evidence for the candidate.

## Corrected decision

Mihomo remains the production engine for the current subscription. The profile
uses Shadowsocks AES-128-GCM with HTTP simple-obfs. Release builds retain the
capability gate that resolves this profile to Mihomo even if Meow or sing-box is
selected. A compile-time diagnostic flag is required to bypass that gate, so a
user preference cannot silently enable an unverified path.

The earlier raw Meow and sing-box 0/10 reports are withdrawn. The old benchmark
always forwarded device port 7890. Meow had its mixed listener disabled, and
sing-box had no mixed inbound at all, so those requests did not establish which
candidate handled the traffic. The benchmark now requires an explicit engine
name, records an engine-owned ingress, and sing-box renders its own loopback
mixed inbound.

## Source-level cause separation

### Mihomo advantage on HarmonyOS

`FlClash-ohos-upstream/core/lib.go` installs `dialer.DefaultSocketHook` and calls
`VpnConnection.protect(fd)` before every outbound connect. It fails closed when
the callback is missing or refuses a socket. The Harmony extension supplies the
callback in `VpnExtAbility.ets`. This prevents an outbound socket from being
captured by the TUN it is trying to serve and makes the failure observable.
Mihomo also uses its own Clash-compatible configuration model and the
`metacubex/sing-shadowsocks2` plus simple-obfs implementation used by its
Shadowsocks adapter.

### sing-box root problem and repair

The TinyPaws sing-box platform bridge previously returned `false` from
`UsePlatformAutoDetectInterfaceControl`, while the generated route had
`auto_detect_interface: false`. sing-box therefore installed no control
function on outbound sockets and relied only on process-wide
`protectProcessNet()`.

On the phone this produced 0/30 timeouts, no visible connection to the selected
node port, and VPN-process RSS growth from 152,560 KiB to 659,440 KiB. TinyPaws
now captures the physical network handle before creating the VPN, enables
sing-box platform socket control, and calls the Harmony native
`OH_NetConn_BindSocket` for each outbound socket. After that change requests
reach the node immediately and RSS remains stable. The current provider still
closes sing-box's Shadowsocks+simple-obfs sessions before TLS completes. Aligning
the HTTP obfs request bytes with Mihomo did not change this result, so the
remaining incompatibility is below DNS/rules and inside the sagernet
Shadowsocks/plugin session path for this provider. Production therefore keeps
this profile on Mihomo.

### Meow result

Meow uses TinyPaws' Rust HTTP simple-obfs wrapper over `shadowsocks-rust`. Its
handshake header order and casing match Mihomo, and it has an upstream
integration test against a standard simple-obfs server. On this device it
completed 9/10 comparable requests with stable process memory. It still relies
on process-wide protection rather than a per-socket callback and one request
timed out, so it does not replace Mihomo for this profile yet. Historical
`UnexpectedEof` records show that this provider can close a session after the
client writes its first encrypted payload; they do not by themselves prove a
Rust memory or DNS defect.

## Comparable physical-device results

All rows below used the same subscription, selected node, target URL, 10 fresh
HTTPS requests, concurrency 2 and an engine-owned port 7890 listener.

| Engine | Success | Median | p95 | Process/RSS | Conclusion |
| --- | ---: | ---: | ---: | --- | --- |
| Mihomo 1.10.0 | 10/10 | 696 ms | 3,987 ms | PID/start stable; +128 KiB | Production baseline passed |
| Rust Meow | 9/10 | 1,889 ms | 7,034 ms | PID/start stable; -2,028 KiB | Functional, below release gate |
| sing-box 1.13.21 after socket binding | 0/10 | — | — | PID/start stable; -2,832 KiB | Provider closes every session immediately |

A separate Mihomo run at concurrency 4 completed 79/100, dominated by TLS
`SSL_ERROR_SYSCALL`; the immediately following comparable run completed 10/10.
This is evidence of node/time/concurrency volatility and is not used as a core
regression claim. Earlier route-proven Mihomo runs remain useful historical
stress evidence: 1,000/1,000 and 9,959/10,000 with stable process identity.

Current comparable reports are:

- `dist/TinyPaws-1.2.7-mihomo-comparable-v2.json`
- `dist/TinyPaws-1.2.7-meow-raw-v2.json`
- `dist/TinyPaws-1.2.7-sing-box-bind-obfs-v2.json`

The 100-request volatility sample is
`dist/TinyPaws-1.2.7-mihomo-v2.json`. Reports contain hashes instead of public
IP addresses or node credentials.

## Implemented product work

### P0

- Source profile, persistent user overrides and generated Harmony runtime config
  are separate. Subscription refresh retains TinyPaws overrides.
- Preflight reports DNS self-reference, unsafe options and changed field paths.
- Status separates VPN establishment, network protection, DNS activity, target
  service probe and public-exit observation.
- Diagnostics export allowlisted lifecycle, OS-exit, connection and process
  evidence without raw YAML, subscription URLs, passwords or logs.
- Process evidence includes bounded RSS/FD/thread samples and a quantified trend
  summary that avoids calling a short window a memory leak.
- The VPN uses the HarmonyOS `dataTransfer` continuous task and one system-owned
  notification path. The obsolete extension notification loop was removed.

### P1

- Per-application trusted/bypass bundle lists reach `VpnConfig`; ANCO/出境易
  containers use the documented device-wide compatibility route.
- Service health can probe a real HTTPS target, fail over after two failures,
  try at most three candidates and observe a five-minute switch cooldown.
- Service-specific DNS policies and user rules survive subscription updates.
- Generic DNS/TLS hostname recovery applies to all domains. The X repair is not
  a hard-coded X-only rule; tests cover arbitrary fragmented TLS hostnames.
- A default-network monitor debounces handovers and reloads Mihomo/sing-box or
  resets candidate connections without creating another notification path.

### P2

- System Live View, service card, portable backup/restore and WebDAV backup are
  present.
- Mihomo and sing-box external-engine bridges are packaged. A capability gate
  keeps plugin profiles on the verified path until each raw candidate passes the
  same physical-device route test.
- The repository includes a repeatable, route-proven Harmony benchmark rather
  than relying on qualitative “usable” labels.

## Release gates

1. 1,000 route-proven HTTPS requests with at least 99.5% success and recorded
   median/p95/p99 latency. **Passed: 1,000/1,000.**
2. Three eight-hour lock-screen runs with no VPN process replacement.
3. Repeated Wi-Fi/mobile-network changes, reconnects and profile reloads.
4. A 24-hour process-evidence window without sustained RSS/FD growth and without
   `APP_KILLED` for the VPN process.

Gates 2–4 are time-based field acceptance and are not claimed by the bounded
delivery run. HarmonyOS can still terminate third-party processes under
system-wide kernel or memory pressure. A continuous task improves scheduling
priority and supplies the correct notification contract; it cannot make a user
process immune to SIGKILL.

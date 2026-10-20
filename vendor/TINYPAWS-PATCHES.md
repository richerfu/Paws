# TinyPaws patches on Meow 0.22.0

The `meow-*` directories are based on the crates.io 0.22.0 sources and selected
by the root `[patch.crates-io]` table. Keep these patches active when upgrading:

| Package | Local behavior |
| --- | --- |
| `meow-common` | OpenHarmony socket protection before TCP connect / UDP bind; fail closed without a protector; physical-network bootstrap DNS during resolver re-entry. |
| `meow-anytls` | OpenHarmony protector registration and protected UDP sockets. |
| `meow-proxy` | AnyTLS bridge, marked-socket protection, eager HTTP-obfs destination handshake, bounded first-flight recovery and socket evidence. Raw socket evidence now wraps `DirectDialer` sockets before the 0.22 stream abstraction erases the socket type. |
| `meow-transport` | Mihomo-compatible HTTP-obfs header order/key spelling, incomplete-header errors and header-only response handling. This code moved here from `meow-proxy/src/simple_obfs.rs` in 0.22. |
| `meow-listener` | Validated benchmark probe IDs carried through the new shared inbound routing path. |
| `meow-tunnel` | Connection spans and relay byte/error evidence, preserving 0.22 admission, cancellation and dial timeout behavior. |

Hysteria2 0.22 already obtains its UDP socket through `meow_common::bind_udp`,
so the old Quinn socket implementation is no longer needed. The protected
socket behavior remains in the active implementation.

The transport push-server test fixture retains the open request body until
the client finishes, preventing a premature `RST_STREAM(NO_ERROR)` from
discarding an otherwise valid queued response in the receive-window tests.

`paws_core` includes the first-flight, socket-evidence and benchmark-probe
regression modules in its host tests. Its `http_obfs` integration tests use
the production `meow-transport` API to check request framing, fragmented and
header-only responses, and truncated-header errors. Run the workspace tests
or this focused regression:

```sh
cargo test --locked -p paws_core --test http_obfs
```

The independent Go bridge and the four patches under `patches/` are retained
from `tinyko:feat/harmony-vpn-resilience-and-ui` unchanged. Native engine
packaging and device acceptance are separate from these host regressions.

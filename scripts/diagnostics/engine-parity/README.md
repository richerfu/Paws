# Isolated three-engine regression

This reusable harness prepares/verifies a debug QA stage and serves bounded
public protocol fixtures. It does not compile, sign, install, operate HDC or
start VPN. Unit tests and management HTTP success are not TUN/device evidence.

## Isolation and artifact gates

- Use the normal bundle/signing identity and a fresh 32-character lowercase hex
  nonce. Both UI and Extension select the QA Core home before initialization;
  never initialize personal Core then change its home in the same process.
- Journal/exit paths share the QA base. Personal widget reads/updates and OS
  backup/restore are disabled. The stage is not an OS network/VPN sandbox.
- Production PawsApp, Gateway and UiCommandSession remain real. Native libraries
  and every rawfile come byte-for-byte from the specified normal HAP; no FakeHost
  or alternate engine library substitutes for production behavior.
- HAR sources, the one declared derived lockfile, all exact-count source spans
  and stage differences are recorded. Unknown source additions or changed spans
  fail closed. No private signing configuration/material is copied to the stage.
- Confirm the normal VPN is fully stopped first. Preserve personal profiles;
  isolation checks use only safe relative-path/stat metadata, never YAML/URLs or
  whole state. Derived startup files may change during normal boot; report that
  scope rather than restoring timestamps or claiming content hashes.

## Prepare and verify

Freeze a debug normal HAP/manifest and its matching host/HAR sources. Set
`NONCE`, `NORMAL_HAP`, `NORMAL_MANIFEST` and reachable numeric `FIXTURE_HOST`:

```sh
python3 -B scripts/diagnostics/engine-parity/stage.py \
  --nonce "$NONCE" --normal-hap "$NORMAL_HAP" --normal-manifest "$NORMAL_MANIFEST" \
  --fixture-host "$FIXTURE_HOST" --http-port 18746 --dns-port 18753 --udp-port 18754
python3 -B scripts/diagnostics/engine-parity/stage.py --verify --stage "$QA_STAGE"
```

The command prints the new stage path. Existing stages are refused, not deleted.
Build/sign separately with the authorized toolchain/identity, then repeat verify
with `--qa-hap "$QA_HAP"`. Exact SO/rawfile verification is mandatory after both
packaging and signing; it does not verify cryptographic trust or runtime isolation.
Only install/run when device ownership is explicit and uncontended.

## Controlled protocol service

```sh
python3 -B scripts/diagnostics/engine-parity/fixtures.py \
  --bind "$FIXTURE_HOST" --advertise-host "$FIXTURE_HOST" --nonce "$NONCE" \
  --internet-dns "$DNS_RESOLVER"
```

One or two explicit numeric Internet resolvers are required; none are guessed.
Ready JSON supplies actual ports and nonce-gated profile/control/event URLs.
The service provides profile/Provider A→B, ETag/304, delay/error controls, HTTP
exact-body echo/1 MiB stream, raw TCP echo, UDP/TCP DNS and ordinary UDP echo.
Two SOCKS nodes forward only to these fixture destinations, with bounded sockets,
bytes, duration and event history. They are not open Internet proxies.

Public startup health stays real HTTPS with a DIRECT route, not an intercepted
response or bypass. Requested→actual fallback is not success for the requested
engine. Verify host UDP reply routing before attributing a timeout to an engine.

## Explicit UI-process probes

All actions require debug, initialized isolated trace, the compiled nonce,
public ASCII case ID and no concurrent probe. Supply only
`pawsEngineParityNonce`, `pawsEngineParityCase` and `pawsEngineParityProbe`:

| Probe | Required evidence |
| --- | --- |
| `http` | Full nonce/case echo from the compiled HTTP endpoint |
| `http-stream` | Exact full fixed 1,048,576-byte body, not a byte-count-only result |
| `dns-upstream` | Actual UDP response from the fixture DNS socket |
| `dns-tun` | Nonce A/AAAA through the fixed QA TUN DNS endpoint and native resolver |
| `udp-echo` | Complete ordinary UDP echo, separate from DNS hijacking |

DNS optionally accepts `pawsEngineParityRecord=A|AAAA`. Targets/sizes are compiled;
Want cannot supply URL, body or configuration. Known OS metadata keys are ignored
without reading values; unknown business parameters are rejected. Deadlines and
socket/request cleanup are bounded. Probes never import/save/select/start/stop.

`--ui-error-probe` is optional and off by default. It admits only
`ui-error-multiline` / `ui-error-unbroken`, arming one compiled public error for
the next real proxies query, once, with a 10-second TTL. It does not issue that
query. This tests Gateway→DomainStore→presenter, not an unknown native cause or
recovery. Readability, final-row reachability, Close/reopen and Retry need actual
checks; a Copy press alone does not prove clipboard contents.

The stage trace contains only whitelisted domain/owner/revision/receipt and
numeric error-category metadata, with bounded records/bytes and explicit
truncation. It has no extra poll/query and stores no arbitrary error/log bodies,
configuration, group names, URLs or credentials.

## Regression and cleanup

```sh
python3 -B scripts/diagnostics/engine-parity/test-stage.py
node --test scripts/diagnostics/engine-parity/test-paths.mjs
node --test scripts/diagnostics/engine-parity/test-ui-error.mjs
python3 -B scripts/diagnostics/engine-parity/test-fixtures.py
```

These use synthetic ZIPs/effect ports or real local protocol sockets; native
compile and device execution are separate. On device, bind every result to the
actual engine/owner and a fresh fixture cursor, test positive and negative cases,
and Stop through the real UI before changing cases. Force-stop is not Stop/join.
Do not infer a packet size from a 1 MiB transfer or claim all features from smoke.
After real Stop/no TUN, restore the preserved normal application without QA Wants.
Pause immediately during manual use; delete only the exact owned QA nonce later.

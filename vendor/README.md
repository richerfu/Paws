# Native engine integrations

All meow-rs packages use the unmodified Git revision
[`52d75ec0fe7eefcc427639ed2bb7ea242dc2781f`](https://github.com/meow-rs/meow-rs/commit/52d75ec0fe7eefcc427639ed2bb7ea242dc2781f).
Its declared version is 0.29.0, and it includes post-`v0.29.0` OpenHarmony and
transport fixes. Paws reports `0.29.0+52d75ec` to distinguish it from the tag.

There are no meow-rs copies in this directory and no Cargo patch table. The
nine former `meow-*` forks have been removed. OpenHarmony socket protection,
HTTP-obfs framing and bounded first-flight recovery now come from upstream.
Paws-only raw socket wrappers, probe-header instrumentation and per-flow info
spans are removed.

Paws no longer patches health, provider, or connection observation interfaces.
Backends without native revision notifications use the ordinary UI refresh
cadence and normal upstream queries. Backends without connection event history
show sampled history, with its limitation visible in the UI; they do not claim
to capture every short-lived connection. sing-box uses its existing public
connection-event subscription instead of a synchronous lifecycle patch. The
extra DNS query observer hooks have been removed from both Go engines.

Read-only Meow previews use upstream rebuild APIs over private snapshots of
provider caches. They do not toggle the process-global offline switch, resolve
DNS upstream hostnames, download providers, or start background health checks.
The original raw configuration is retained and a runtime start reloads it
through the ordinary upstream loader. Preview construction is not a running
resolver or a substitute for runtime validation.

The bounded GeoSite category decoder belongs to
`paws_engine_go::config::geosite`, which compiles sing-box matchers. MRS decoding
uses upstream public readers after checking the application's byte budget.
The original Meow MIT notice is in [LICENSES/meow-rs/LICENSE](../LICENSES/meow-rs/LICENSE).

## Go bridges and remaining patches

`mihomo-bridge`, `sing-box-bridge`, and `paws-engine-observation` are application
integrations and remain required. They provide the native entry points,
OpenHarmony networking, authenticated management transport, and bounded logs.

Two patch files were removed: the sing-box benchmark probe, and the Mihomo TUN
FD patch whose fix is already present in the owned bridge source. The two
management patches have also been reduced to remove redundant observer hooks.
Remaining patches cover OpenHarmony support, HTTP-obfs protocol behavior,
provider/runtime integration, and correct lifecycle handling. They are applied
to exact archived upstream sources, never to mutable user checkouts.

Paws uses the prebuilt go-patch compiler without modifying Go or gVisor. See the
[native engine build guide](../docs/native-engines-build.md) for source pins,
patch ownership and build commands.

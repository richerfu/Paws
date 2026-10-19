# TinyPaws vendor notes

Source: crates.io `meow-listener` 0.21.2, https://github.com/meow-rs/meow-rs (MIT).
The published crate is retained with its source, build script and tests. Cargo registry cache metadata is excluded; the upstream MIT notice is retained in LICENSE.

TinyPaws adds a diagnostic-only CONNECT probe ID event in `http_proxy.rs` and strict parsing in `benchmark_probe.rs`. It activates only when compiled with `PAWS_RAW_ENGINE_DIAGNOSTIC=1`, on the loopback listener named `paws-meow-benchmark`. It records only a validated `X-TinyPaws-Probe` ID and the device-side source port; arbitrary header values, authorization and payload are never logged by this addition. Duplicate probe headers are ignored.

Format: `tp-<16 lowercase hexadecimal run ID>-<decimal request ID>`. Request IDs are 1 through 1000000000 without leading zeros. The maximum identifier length is 30 bytes.

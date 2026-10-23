# Paws patch provenance

This is `meow-dns` 0.22.0 from the exact crates.io archive pinned by Paws.
Archive SHA-256: `a21504123338c027f034cb86685f53f91ecd9e4be0970764155d133ba7ad41b6`.
Every original cached source file was compared byte for byte with that archive
before copying. The package version and production dependencies are unchanged.
The archive omits its MIT license file; LICENSE is copied from the same-pinned
meow-config package/repository license already included by Paws.

Paws changes:

- A matched nameserver policy is selected before global domain fallback gates
  and is terminal for A/AAAA and non-address exchanges. Its positive, negative,
  no-data and transport-error outcomes do not enter global main/fallback pools.
  This matches the pinned Mihomo resolver contract and avoids filtering an
  explicitly selected policy response through the global GeoIP fallback filter.
- Unmatched names retain the existing global main/fallback behavior.
- `src/policy_tests.rs` exercises real UDP upstreams through production DnsServer
  entry points for A, AAAA and TXT, including policy outcomes and no-policy
  fallback controls. Related pipeline comments are updated.

No registry checkout is modified. Root Cargo applies this owned source with a
central `[patch.crates-io]` entry; no dependency upgrade is required.

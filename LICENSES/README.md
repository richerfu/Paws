# Bundled engine licenses

These files preserve the notices of the exact upstream revisions used by the
native engine build scripts. They supplement the Paws-authored source license
in the root [LICENSE](../LICENSE).

| Component | Packaged source revision | License files |
| --- | --- | --- |
| NSH runtime and Gallery UI theme | Local `e10ae2c47b3d711c67485ed49f6b76633ebebd93` plus recorded local changes | [Apache License 2.0, upstream text](nsh/LICENSE-APACHE) |
| Mihomo 1.10.0 | [`8597778c7df410ebe88c57e59eaab32d203156df`](https://github.com/chen08209/Clash.Meta/tree/8597778c7df410ebe88c57e59eaab32d203156df) | [GPL-3.0, full upstream text](mihomo/LICENSE) |
| sing-box 1.13.21 | [`628cb31ffa79cffffd34c2f9cde6cae044e4fc12`](https://github.com/SagerNet/sing-box/tree/628cb31ffa79cffffd34c2f9cde6cae044e4fc12) | [Copyright and upstream notice](sing-box/LICENSE), [full GNU GPL v3 text](sing-box/COPYING) |

`mihomo/LICENSE` and `sing-box/LICENSE` are copied verbatim from each pinned
source revision. `sing-box/COPYING` contains the complete GNU GPL v3 text,
which is also supplied by the pinned Mihomo source. The sing-box notice permits
version 3 or later and retains its additional naming/association notice.

The Go bridge and HarmonyOS patches live in `vendor/mihomo-bridge`,
`vendor/sing-box-bridge`, and `patches/`. See
[third-party components](../THIRD_PARTY_NOTICES.md) and
[native engine build instructions](../docs/native-engines-build.md) for the
source and adaptation details.

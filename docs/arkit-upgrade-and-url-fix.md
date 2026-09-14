# Arkit upgrade, shadcn alignment and About links

Base: Paws `8126443`. Arkit is pinned to upstream main commit
[`1d4163f2168a49aba1a6fd8ea6ca81406212c4e9`](https://github.com/richerfu/arkit/commit/1d4163f2168a49aba1a6fd8ea6ca81406212c4e9),
including all 14 resolved Arkit crates. The removed `arkit_dom` crate is no
longer in the lockfile. Dioxus and NAPI minimum versions now match Arkit's
`0.7.10` / `1.2.0`; those versions were already resolved in the previous lockfile.
As verified on 2026-09-13, the paired Rust/ArkTS ability package remains at the
latest published `1.0.0-beta.2`, and URL/files plugins remain at latest published
`beta.1`.
Unreleased ability git snapshots and unrelated VPN-engine dependencies were
not mixed into the published ArkTS bridge. About's Arkit revision is generated
from the workspace dependency pin at build time, rather than maintained twice.

## URL failure

`ohos.url` is a typed bridge capability, not a missing `@ohos.url` package to
install. ArkTS already declared `new LazyPlugin(() => new UrlPlugin())`, but
the Rust `#[entry(plugins = [...])]` list omitted `UrlBridgePlugin`. Native
initialization therefore did not advertise/install the capability, and
`OpenHarmonyApp::open_url` could not resolve it. `FilesBridgePlugin` had the
same omission and is now registered too. Both are registered exactly once;
the typed URL API still propagates actual browser/platform failures.

Regressions check both sides of entry assembly, exercise the real installed
ArkTS URL plugin against both production About URLs, and verify invalid URL
and system-open failure handling. They do not mistake mocked `openLink` for
device browser-launch proof.

## Style and API migration

The reference is Arkit's native shadcn **New York** implementation, not a
pixel-identical web/Tailwind port. The official
[theme-token conventions](https://ui.shadcn.com/docs/theming) guide surface /
foreground pairs, light/dark modes, borders, typography and radius reuse.

- Common buttons inherit upstream variants and New York 36/32/40vp sizing.
- Segmented filters use upstream `TabsList` / `TabsTrigger`; dialogs use
  upstream `Dialog` for panel, close affordance, motion and dismissal.
- Removed `Form` / `FormItem` APIs are replaced with `FieldGroup`, `Field`,
  `FieldLabel` and the existing explicit app-owned drafts and validation.
- Cards use upstream `CardTitle` / `CardDescription`. All page font sizes and
  common radii use shared tokens; compact text is at least the 12vp XS token.
  Detached activity icon actions use the upstream 32vp compact target.
- Existing flat mobile card surfaces and safe-area/navigation geometry remain
  intentional adaptations. Success/warning are shared app semantic extensions;
  transparent fills, square joined-menu edges and tiny chart dots are not
  replaced with rounded card styling.
- Six virtual lists use the new `use_virtual_items` / `VirtualItemStamp` API.
  Request/connection IDs and archive filenames stay stable while visual
  revisions change. Duplicate equal log records receive occurrence IDs, not
  a collision-prone hash identity. Proxy/resource detached rows continue to
  observe explicitly passed domain/operation Signals and theme palettes.

## Verification

Validation logs and device captures are retained locally under `smoke-logs/`;
they are excluded from Git.

- `cargo test --workspace --locked`: **417 passed**, no failures, including
  all doc-test targets (`arkit-upgrade-workspace-final.log`).
- `cargo check -p paws_ui --lib --locked`, formatting and `git diff --check`:
  passed. The native/ArkTS release HAP build passed
  (`arkit-upgrade-release-final.log`).
- `node scripts/test-vpn-platform.mjs`: **35 passed**, including the actual
  installed URL plugin's About-URL and error-path tests.
- A public-test-signed release was installed on the independent
  `Paws_Final_Release` HarmonyOS emulator (`127.0.0.1:15563`). All **14 pages**
  passed rendered-title/no-panic assertions; each light-theme screenshot was
  visually reviewed (`arkit-upgrade-pages-final/`). This is not a physical-device
  distribution signature; the later physical-device installation is recorded
  separately below.
- Settings/About alignment and revision-truncation smoke passed
  (`arkit-upgrade-settings/`). Both **actual About buttons** launched Huawei
  Browser and rendered their corresponding GitHub repositories, verified by
  screenshots and system UI trees (`arkit-upgrade-device/url-browser-ready.*`
  and `url-meow-ready.*`). This confirms the complete native-to-ArkTS-to-system
  URL path, not just the mocked test boundary.
- Local-file import opened the actual system document picker
  (`arkit-upgrade-device/files-picker-ready.*`); no file was selected/imported.
  The picker remained open during inspection beyond the bridge's existing
  60-second deadline, so the real timeout was surfaced in the import dialog.
  Picker launch is verified; a completed file-import/cancel round trip is not
  claimed by this acceptance. The test app was restarted without clearing data.
  The new upstream import dialog was also visually
  checked (`arkit-upgrade-interactions/import-dialog.jpeg`).
- **Dark-theme device acceptance remains incomplete.** Subsequent diagnostics
  identified a separate `h_strongswan` UI automation using the same `15563`
  target and explicitly force-stopping Paws. The final attempted dark capture
  showed that other app, despite an earlier Paws layout assertion, so it is
  rejected as acceptance evidence. No other task/process was stopped. Further
  UI interaction was suspended to avoid competing for the device; the temporary
  screen-timeout override was restored. Paws' test-device theme may remain Dark
  until an exclusive device session can restore System and finish this check.
  Production light/dark token migration is implemented, but full dark visual
  acceptance must not be inferred from the completed light-page checks.

VPN service activation is never part of emulator UI tests. VPN business logic
was not changed by this upgrade.

## Physical-device release installation (2026-09-14)

After the user refreshed the local DevEco signing configuration,
`HAP_BUILD_MODE=release NATIVE_PROFILE=release scripts/package-signed-hap.sh`
completed successfully. The resulting `entry-default-release-signed.hap`
contains `com.richerfu.paws` version `1.0.0` with `debug=false` and was installed
on the ALN-AL10 using `hdc install -r`.

The install succeeded, and before/after bundle metadata retained the same UID
and original installation time while the update time advanced. The application
launched successfully; the rendered home page showed the existing configuration
and node groups, with no rendered panic. VPN was not started. This verifies
installation and startup, not the pending full dark-theme acceptance.

Evidence: `physical-release-20260914-final-build.log`,
`physical-release-20260914-install.log`, the before/after bundle metadata and
`physical-release-20260914-ui.*` under `smoke-logs/`.

Release HAP SHA-256:
`33c0b5d567c0421247d9818e19b4ae99f9e19f64fa8a1ad5efe8e34ea6e7a368`.
Machine-local signing configuration, certificates, private material and binary
artifacts are excluded from this change's commit.

# Paws native UI

## Production ownership

`App.tsx` mounts the production PawsApp. The paired NSH CLI compiles it to a
bundle loaded by HarmonyOS `NshSurface`/`ContentSlot`. NSH owns native nodes,
layout, input, recycling, navigation and system Back.

`src/client.tsx` owns command receipts and domain synchronization;
`src/domain-store.ts` owns subscribed data and revisions. Pages own drafts and
foreground interests. The `paws.core` component connects to the host Gateway,
then `paws_app::Application` through the thin Rust N-API binding. Configuration,
persistence, engine policy and VPN ownership remain outside the view layer.

Shared controls and theme tokens live in `src/components.tsx` and `src/theme.ts`.
Lucide icons use native Canvas paths; their license is `assets/LICENSE.lucide`.
Do not pass SVG strings to the OHOS bitmap decoder.

## Communication and lifetime

- Subscribe before querying. Fourteen typed domains and separate preferences
  replace aggregate UI snapshot transport; internal Core state reads still exist.
- Keep config/status interests; request large detail data only while needed.
  Retained routes release foreground interests without discarding drafts.
- Allow one query per key, coalesce invalidations, and reject obsolete generation,
  owner or revision results. Missing data exposes loading/error/Retry.
- ACK is not completion. Writes retain their receipt/query barrier; a failed
  follow-up read reports a warning and never replays a committed mutation.
- Preview checks stay local and do not disable global Settings. Late results
  cannot overwrite a newer choice, clear newer busy state or open a Dialog.
- Guard page lifetimes; persist committed application work independently of a
  closed Dialog. Stop can supersede Start without adopting a later owner.
- Preserve fade, safe areas, exact full text and native input semantics. An IME
  candidate is not a committed Input value. Copy source is not a display prefix.

## Build and host checks

Run from the repository root. Build the paired SDK with
`scripts/build-nsh-ohos.sh`; the standard CLI is `dist/nsh-sdk/release/nsh`.
A compatible alternate CLI can be selected with `NSH_BIN` or `--nsh-bin`.
Choose an owned, untracked output directory before running:

```sh
NSH_BIN=dist/nsh-sdk/release/nsh
QA_OUTPUT=target/ui-checks
python3 ui/nsh/run-tests.py --nsh-bin "$NSH_BIN" --log-dir "$QA_OUTPUT/standard"
"$NSH_BIN" build ui/nsh --release -o "$QA_OUTPUT/bundle"
python3 ui/nsh/tests/run-engine-preview-tests.py --nsh-bin "$NSH_BIN" --output-dir "$QA_OUTPUT/preview"
python3 ui/nsh/tests/run-page-error-tests.py --nsh-bin "$NSH_BIN" --output-dir "$QA_OUTPUT/errors"
python3 ui/nsh/tests/run-settings-mtu-tests.py --nsh-bin "$NSH_BIN" --output-dir "$QA_OUTPUT/mtu"
node --test scripts/test-vpn-platform.mjs
```

The fixtures drive actual production components/client with public synthetic
receipts. Runners require delivered events, expected state and commands; runtime
errors, missing taps and timeouts fail. Geometry tests must check sibling overlap,
vertical clipping and end-of-scroll access as well as window bounds.

## Real-device boundary

Headless traces do not establish native pixels, IME, clipboard contents, system
bars, rendered-frame timing or engine traffic. Font-scale emulation is not OS
font delivery, and worker time is not FPS. Validate those paths on the exact
candidate separately, using a clearly authorized operation window.

Do not read/edit personal YAML or start a personal VPN for synthetic UI checks.
Pause automation during manual device use. Preserve real error/Retry feedback;
unsupported or unavailable native fields must not become empty successful data.
The development contract is [NSH architecture](../../docs/nsh-three-layer-architecture.md).

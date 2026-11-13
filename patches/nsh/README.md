# NSH compatibility patches

`system-alert-dialogs.patch` backports the Rust implementation from
[`nsh-platform-harmonyos` PR #23](https://github.com/huacnlee/nsh-platform-harmonyos/pull/23),
commit `726117f3b2e5b55dc7b251f46783f39985643045`, to Paws' pinned NSH 0.4
checkout. It retains the window's UIContext and calls its system
`showAlertDialog` / `showActionSheet` methods through N-API. ArkUI owns the
layout, colors, typography, buttons and animations; no ArkTS code is added.

The backport contains the four production Rust files. The newer platform's
device-test APIs are absent from NSH 0.4, so its development-only dialog
inspection helpers and test runner changes are omitted.

Build staging applies these patches in filename order without changing the
pinned checkout. Patch contents participate in the stage fingerprint and
the patched source participates in the SDK and HAP provenance. An incompatible
`NSH_ROOT` fails at patch application. Remove the backport when the pinned SDK
includes the system dialog implementation.

System alerts follow the window's appearance. The API has no destructive
button role or programmatic close method: retiring a surface invalidates its
callbacks, and a displayed alert remains dismissible until the user dismisses
it or the window closes. Results and queued alerts advance after the native
exit animation completes.

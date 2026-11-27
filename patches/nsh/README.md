# NSH compatibility patches

No active patches are currently applied to the pinned NSH 0.6 (`scripts/nsh-revision`)
or `nsh-platform-harmonyos` (`scripts/nsh-harmonyos-revision`) checkouts.

The earlier NSH 0.4 backports (`surface-resize-environment.patch` and
`system-alert-dialogs.patch`) are included natively in `nsh-platform-harmonyos` 0.6.

Build staging (`scripts/prepare-nsh-build-source.py`) continues to apply any
`*.patch` files placed in this directory in filename order without mutating the
pinned checkouts, and includes patch contents in the stage fingerprint and HAP
provenance.

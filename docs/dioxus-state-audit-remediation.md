# Dioxus state and session audit remediation

Baseline: Paws `8d4579c`, Dioxus 0.7.10, Arkit `7ede4e0`.
Comparison: local h-openconnect `e95fa26` (session/route ownership),
`db673d3` (editor scope and focus), and `b4acdec` (system appearance).

Implementation branch: `fix/dioxus-state-session-audit`.
The exhaustive mapping of all 94 former UI actions is in
[ui-action-migration-ledger.md](ui-action-migration-ledger.md).

## Design basis

Dioxus does not prohibit shared state or command dispatch. The problem here was
putting unrelated reactive values, editor drafts and operation progress into
one subscribed object, then routing every interaction through a universal
action interpreter. The replacement keeps durable domain invariants in Core,
uses independently subscribed signals for UI projections, and keeps disposable
drafts and query ownership at component scope. This follows the official
[signal subscription model](https://dioxuslabs.com/learn/0.7/essentials/basics/signals/)
and [typed context pattern](https://dioxuslabs.com/learn/0.7/tutorial/state/).
Derived data uses memo boundaries where useful; a signal/context wrapper alone
does not make an expensive root render granular.

[Async resources](https://dioxuslabs.com/learn/0.7/essentials/basics/resources/)
are suitable for replaceable reads, not automatically for durable writes:
dropping a UI future cannot undo partially persisted configuration. Network
preparation may be cancelled; a started commit must finish or roll back, while
its UI callback is still guarded by the initiating component/root lifetime.
The pinned Arkit renderer also polls Dioxus futures outside a Tokio executor,
so Tokio-dependent Core work must run on its explicit runtime.

The local h-openconnect changes provide project-specific comparisons, not an
API to copy verbatim: `e95fa26` motivates explicit VPN session/resource ownership,
`db673d3` motivates local editor state and simpler focus/scroll boundaries, and
`b4acdec` motivates event-driven system appearance with observable application
acknowledgement. Platform capabilities are checked against this project's pinned
SDK instead of assuming that another VPN implementation has identical support.

This ledger tracks the implementation and behavioral acceptance of the state,
asynchronous ownership, persistence, and VPN lifecycle audit. A code change is
not evidence that a device scenario has passed. Verification results below must
identify what was actually run.

## Baseline findings and implemented plan

1. **VPN ownership and teardown (high risk):** native worker termination was not
   a reliable terminal event; late extension callbacks could act on replacement
   resources. The fix gives each attempt an identity, joins native shutdown,
   guards every platform callback, and waits for exact-owner cleanup before
   replacement. Remote death/freeze is handled by a monotonic watchdog, not by
   trusting a previously successful snapshot indefinitely.
2. **Persistence and transactions (high risk):** corrupt storage could appear
   as a successful empty/default store; several writes, activation/reload and
   rollback steps were separately observable. The fix fails closed, preserves
   original files and errors, serializes configuration commits, checks the
   initiating revision, and returns an atomic projection receipt. Imports now
   separate cancellable preparation from durable commit/rollback.
3. **Reactive state (performance and maintainability):** an aggregate UI state
   mixed runtime snapshots, preferences, drafts and pending flags. Broad action
   dispatch and full-state cloning made unrelated updates invalidate unrelated
   views. Typed domain signals, separate operation signals, local drafts and
   memo/component boundaries replace this arrangement.
4. **Async ownership and ordering (correctness):** delayed reads/saves could
   target a different profile or update a disposed page. Checked identities,
   revisions, request generations and scope-owned queries prevent these writes;
   durable commits intentionally survive disposal without reviving callbacks.
   Tokio work runs on the application runtime, with bounded UI delivery.
5. **Draft and error handling (data integrity):** settings could follow the
   newly active profile while retaining old edits; some failures were silently
   defaulted or cleared by unrelated updates. Drafts now retain their profile,
   revision and dirty sections, surface conflicts, and require explicit reload.
   Bootstrap, projection and preference errors have distinct ownership.
6. **Unnecessary indirection and misleading fallbacks:** universal UI action
   plumbing, modal content hashes/fixed transition sleeps, duplicate reloads
   and duplicate telemetry sampling are removed. Malformed VPN options,
   unsupported capabilities and unconfirmed platform results are explicit;
   cancellation uses typed outcomes or documented platform codes, not message
   substring matching.
7. **Acceptance quality:** source-string tests and simulated platform calls
   were insufficient evidence for routing or service cleanup. They are now
   complemented by real Dioxus runtime tests and separate device acceptance;
   negative automation checks cannot pass by catching their own assertion.
8. **Device-discovered stop/operation race (fixed):** the v2 QEMU run
   removed the TUN and extension process before asynchronous cleanup could
   acknowledge completion. UI telemetry then cleared Stop pending even though
   the bridge operation was still waiting; a subsequent Start queued behind
   that stop and failed two minutes later. The verified correction
   requests cooperative cleanup before OS teardown, separates operation
   ownership from session telemetry, and preserves queryable operation
   receipts across uncertain transport results. Neither a newer status
   revision nor `vpn_running=false` proves operation completion.
   Exceptional recovery requires confirmed OS stop plus release of the exact
   owner-held kernel lease; PID existence or absence is not cleanup proof.
   Read/parse/permission errors remain unknown. Final QEMU complete-stop,
   restart and crash-recovery scenarios passed with ordinary-UID TUN traffic.
9. **Cold UI ownership (fixed):** same-process root/Plugin replacement is
   distinct from creating a new UI process. A new Core creates an empty ashmem
   lane while an existing Extension can still own the previous lane. A blank
   local attempt therefore does not prove that no system VPN owner exists.
   The verified correction retains only the exact attempt and
   issuer/extension boot, PID and process-start identities in a strictly read ownership journal,
   with cross-process compare/update/delete protection. It does not persist or
   restore the aggregate runtime state. Late, not-yet-adopted Wants must also
   reject an issuer without its exact held lease. Host regressions cover a
   still-held old owner. QEMU cold-process acceptance recovered an old Attached
   journal only after OS stop and exact lease release, then passed TUN traffic.
   On this device, killing the UI also killed its Extension; this is not a
   device claim that an Extension survived its UI process.
10. **Sandbox-hidden process identity (device-discovered, fixed):** v3 QEMU
    rejected the live UI issuer because the application's proc mount uses
    `hidepid=invisible`; peer `stat` access returned NotFound even though the
    same PID/start-time/boot identity was alive when observed by the device
    administrator. Reading one's own proc entry does not establish visibility
    of peers. The [kernel proc documentation](https://www.kernel.org/doc/html/latest/filesystems/proc.html)
    explicitly describes invisible process directories. The correction uses an
    exact owner-held kernel file lock for cross-process ownership lifetime,
    retaining identity fields for fencing. No missing/denied read is accepted
    as death or successful cleanup. Final QEMU start, stop, freeze/death and
    cold-process recovery revalidation passed under the same sandbox.
11. **Pre-native startup classified as stopped (device-discovered, fixed):**
    v4 accepted the exact owner after a real first-authorization dialog had
    remained open for over 60 seconds. The new pre-create lifecycle check then
    interpreted `NativeVpnLifecycle::Stopped` as disconnected before native
    startup had even begun, and immediately cleaned up the valid Pending
    attempt. The correction keeps an attached Pending/starting attempt in
    Starting; explicit Stop/Failed/Cancelled still take precedence. Real Core
    begin/bind/tick and ArkTS create-path regressions pass. QEMU fresh
    authorization waited approximately 62 seconds, then connected and passed
    ordinary-UID TUN traffic without replacing the pending attempt.
12. **Exported debug Want control plane (fixed):** the exported
    EntryAbility accepted profile import/activation, VPN auto-start and network
    diagnostic automation parameters in release builds. This pre-existing
    interface was not an authenticated UI interaction. The correction gates
    automation with the read-only `context.applicationInfo.debug === true`,
    supplied by the platform, never a Want parameter. This is distinct from
    signing-profile type; see the official
    [ApplicationInfo definition](https://raw.githubusercontent.com/openharmony/docs/master/en/application-dev/reference/apis-ability-kit/js-apis-bundleManager-applicationInfo.md).
    v6 release-device probes covered both onCreate and onNewWant: the profile
    index hash was unchanged, with no VPN/echo/import side effects. Actual
    EntryAbility wiring and debug/release behavior have platform regressions.
13. **Completed cleanup retained a lease after notification failure (fixed):**
    exact journal deletion could succeed, followed by a failed IPC
    state publication before dropping the ownership leases. A retry then saw
    cleanup complete but a new Start encountered its own stale held lock. The
    correction releases exact leases after proven cleanup and successful exact
    deletion, before fallible notification. The error remains visible;
    unfinished begin/attach leases remain held. Three boundary fault-injection
    regressions cover normal cleanup, confirmed-stop recovery and unattached
    dispatch failure, including same-process replacement and stale-Want rejection.

## Acceptance ledger

### Final lifecycle and UI operation design

- Session telemetry projects lifecycle/running state only. A separate typed
  operation signal owns the UI operation ID, action, initiating owner and
  AwaitingBridge/Confirming/Unconfirmed phase. Telemetry cannot settle commands.
- The bridge submits once with a process-unique request ID, retains the exact
  operation receipt, and supports repeatable read-only lookup. An uncertain
  result stops the indefinite spinner but blocks another Start. The user may
  explicitly recheck the same receipt or request a compensating Stop; failed
  compensation preserves the prior uncertainty. Superseded `false` is not
  converted to successful Stop acknowledgement.
- Same-realm Plugin instances share one VPN operation queue. Core intent epochs
  reject stale queued work, and a scoped OS-stop in-flight fence blocks new
  starts until the exact system call settles. A rejected system stop retains
  its claimable fence. These are small VPN-specific lifecycle/transport
  protocols, not a replacement universal UI dispatcher.
- A bounded, versioned ownership journal stores only attempt and exact
  boot/PID/start-time identities. Atomic file replacement and a nonblocking
  cross-process lock protect Pending/Attached/Stopping compare-and-swap
  transitions. Unknown, unreadable, malformed or mismatched records fail closed.
  Stopping prevents a late, not-yet-adopted Want from attaching. An attached
  owner must acknowledge full native/platform teardown or release its exact
  ownership lease after confirmed OS stop. Missing is not a wildcard for
  deleting another owner.
- Cooperative stop gives the live Extension a chance to drain native work,
  pending create/protect callbacks and the platform connection before OS
  teardown. Exceptional recovery polls the exact owner lease within a bounded
  deadline; a silent or frozen process does not release the lease. Each process
  acquires a fixed-inode lock before publishing its strict identity record and
  journal transition; another owner is never inferred from a busy lock alone.
  The journal is checked before reusing an issuer inode so an old Pending
  record cannot observe a new holder with the old header. Normal full cleanup
  or process exit releases ownership; an unlocked file is not by itself a claim
  that the process died. Cross-process recovery does not depend on proc
  visibility. Linux/OHOS read only their own boot UUID/start time; macOS host
  identities use bootsessionuuid and native process information.
- Normal Cargo integration tests execute the production pure VPN operation
  model directly; no duplicated state machine or temporary test harness is
  needed. Native process, journal CAS and actual ArkTS ordering have separate
  regression coverage.

| Audit concern | Required resulting behavior | Verification status |
| --- | --- | --- |
| Native TUN termination | Worker completion publishes the current session's terminal state and cause; shutdown completion is observable. | Pass: final host lifecycle regressions and v7 QEMU exact native-join/connection-destroy ACK followed by OS-stop confirmation; TUN/PID absent and the next connection transfers ordinary-UID traffic. |
| Session ownership | Stale Wants and async completions cannot replace IPC, mutate a new connection, or clear its protection; cleanup precedes replacement. | Pass: exact-owner journal/lease/late-Want and publish-failure host regressions; v7 QEMU duplicate system delivery, exact Stop, replacement, freeze/death recovery and cold-UI journal recovery. |
| Remote liveness | Frozen or dead extension state becomes unavailable/terminal using monotonic observation and wake grace. | Pass: v7 QEMU bounded 25-second freeze becomes Failed, remains terminal after CONT; SIGKILL also becomes Failed. Host monotonic/wake-grace and exact-lease regressions pass. |
| Persistence failures | Corrupt/unreadable storage is never presented as an empty successful store or moved silently to another root. | Pass: corrupt/missing/dangling index and referenced YAML tests; log I/O failure injection. |
| Configuration transactions | Resource/version checks cover writes and rollback; rollback failure is reported without a false recovery claim. | Pass: checked stale-write/import/rule tests and primary + secondary rollback failure injection. |
| Platform configuration | Malformed options fail explicitly; bypass capability is either applied with its actual meaning or reported unsupported. | Pass: strict pre-sanitization YAML/port/type/alias tests, ETS validation, legacy-read/new-write/start rejection; rendered read-only capability explanations. |
| Reactive granularity | Editor input, preferences, configuration and telemetry do not invalidate one application-wide state signal. | Pass: real Dioxus independent-signal/same-value tests and production wiring checks. |
| Render work | Rendering avoids complete state clones and repeated disk initialization; expensive derivations have suitable component/memo boundaries. | Pass: one-time initialization and real Dioxus domain/detached-tree regressions; final device Resources and Logs render without panic. Bounded telemetry projection still clones its domain payload; this is not a zero-clone claim. |
| Snapshot purity | Reading a snapshot does not sample traffic, mutate history, synchronize configuration or launch refresh tasks. | Pass: snapshot-purity and telemetry-publish behavior tests; native publication uses a narrow DTO. |
| Async result ordering | Stale resource/query/snapshot completions do not overwrite current state or commit stale edits. | Pass: production VPN operation-model tests keep receipt ownership independent of telemetry, preserve uncertainty and fence old callbacks; two-Plugin ArkTS regressions preserve false/superseded results and stop ordering. Resource/query generation and device late-download cancellation also pass. |
| Task ownership | Page queries, application subscriptions and durable core transactions have explicit owners and cancellation/error behavior. | Pass: Dioxus/Tokio drop-before/after-poll, remount and bounded-backpressure tests; two-phase import/commit tests. |
| Form identity | Drafts retain profile identity and baseline revision, with explicit handling of external changes and save progress. | Pass: five draft-model tests; earlier 15557 UI retains dirty port across external activation, disables save with an explicit conflict, and reloads only after the user chooses it. Earlier rendered save/YAML rollback checks also passed; final frozen UI passed the new 15563 page/settings suite. |
| Obsolete abstractions | Dead dialog content hashes, fixed modal transition sleeps and universal UI action plumbing are removed. | Pass: production code scan and exhaustive 94-action migration ledger. |
| Existing behavior | Notifications, first authorization, Want redispatch, root replacement, virtual lists and import cancellation remain covered. | Pass: v7 QEMU first authorization beyond 60 seconds, real duplicate delivery and notification-permission acknowledgement; final 15563 Resources/Logs plus Dioxus root/drop/import cancellation regressions. Release automation is deliberately rejected and covered on both EntryAbility entry paths. |

## Verification record

### Final v7 host and UI acceptance (2026-09-12)

The final frozen production implementation passed one complete
`cargo test --workspace --locked` invocation: **409 passed, 0 failed**, exit 0.
Breakdown: Core **153**, Model **3**, Profile **76 + 1 integration**, UI **138**,
VPN **38**. `node --test scripts/test-vpn-platform.mjs` passed **34/34**.
`cargo fmt --all --check`, `git diff --check` and smoke-script syntax checks
passed. `cargo clippy --workspace --all-targets --locked` completed successfully
with advisory warnings; this is not a warning-free or `-D warnings` claim.
Logs: `smoke-logs/state-audit-v7-workspace-final.log`,
`state-audit-v7-platform-final.log`, `state-audit-v7-clippy-final.log` and
`state-audit-v7-build-final.log`.

Final release packages (`app.debug=false`) passed signature verification and
share native payload SHA-256
`a1ec49bbdf3732eb1af27e3e1f31e75a0b2eefc70cd5a8421d12f0945b465568`:

- unsigned: `435707c420c7c7545e53b87f6307b9ffb0f916f3d0cde13e633e3db3f033945d`
- new emulator 15563: `97f2e19141ed8b2cb2a87782e7b50727ab39644ec1c165bd039abc48977936d3`
- new QEMU 5563: `46e5a9644c5b0fcf45582fe67c486472ebfd6853cd0922302e40ca5eeb1d3f34`

A new independent emulator, **Paws_Final_Release (15563)**, passed **14/14 pages**
and the Settings/About navigation/version suite. A sol/xhigh reviewer inspected
all 14 final screenshots at original resolution: no blocking overlap, clipping,
blank screen, error page or detached-list rendering issue. The profile was
imported through the actual URL dialog. **No VPN was started on the emulator.**
Evidence: `smoke-logs/emulator-final-release/v7-pages/` and `v7-settings/`.

Automation-dependent smoke scripts now reject release HAPs before touching a
device. The subscription script requires and installs the same debug HAP whose
manifest it checks. Release rejection, missing-HAP rejection and shell syntax
checks passed. Normal non-automation release smoke remains supported.

### Final v7 QEMU acceptance (2026-09-12)

All final VPN scenarios ran on a **new isolated QEMU 5563**, not the DevEco
emulator. Paws UID was `20010041`; an unlaunched normal-APL test fixture allocated
ordinary application UID `20010042` with INTERNET permission. The TCP probe drops
to that UID before opening its socket; neither root nor Paws-excluded traffic is
counted. The fixture's own VPN service was never launched.

Evidence root: `smoke-logs/qemu-state-audit.AMa5eh/v7/`.

- **Fresh authorization:** request 00:55:09.417, Allow after 00:56:14,
  owner bound 00:56:15.881, start complete 00:56:16.685. Attempt
  `1789145709419898341-1` survived the >60-second authorization wait; duplicate
  OS delivery was ignored and created one TUN. `start-pending.pibTHN/`.
- **Real traffic:** `verify.aUrTzF/` echoed 20 bytes to
  `192.168.3.28:62064`, local `172.19.0.1`, UID `20010042`.
  TUN RX bytes rose 0→272 and TX 448→688. This is payload/routing evidence,
  not merely a connected label or root socket success.
- **Complete Stop:** request 00:56:51.844, exact native-join/connection-destroy
  ACK .920, OS onDestroy .980, exact-attempt cleanup confirmation .981.
  No Extension PID/TUN remained. `stop.sj5Cgh/`. The initial notification
  permission dialog was explicitly granted and logged enabled at 00:56:50.080;
  the earlier disabled-notification error is retained, not hidden.
- **Restart and freeze:** new attempt `1789145836326211568-2` connected;
  SIGSTOP for 25 seconds produced Failed, and CONT plus seven seconds did not
  revive it. Explicit recovery `1789145875530240878-3` passed ordinary-UID
  23-byte TUN echo. `start.3F9t36/`, `freeze.MPyiA6/`, `recover.5udjA8/`.
- **Process death:** complete SIGKILL rerun `kill.XNhsak/` became Failed;
  `recover.sHGImd/` confirmed OS stop and exact-owner recovery before new
  attempt `1789145990379306808-5` connected and echoed 23 bytes through TUN.
  The earlier driver-interrupted run is excluded; see `driver-interruption.txt`.
- **Cold UI:** killing UI 2012 also caused the OS to terminate Extension 3764,
  while preserving its Attached journal. New UI 4022 confirmed old-owner cleanup
  at 01:00:11.575, then established `1789146011575837526-1` at 01:00:12.295;
  ordinary-UID 20-byte TUN echo passed. Final exact Stop confirmed at
  01:00:19.323. `cold-ui.5otV0Z/`, `start.5eCg9w/`, `verify.7AQ9BQ/`,
  `stop.Jmtiwj/`. This does not claim a live Extension survived UI death;
  held-owner recovery safety is covered by host regressions.
- **Release gate:** external import/auto-start/echo Wants through onCreate and
  onNewWant left the profile index hash unchanged
  (`181f2310163ae650b0af9c115105559cf6a1eba2af3bdbe5b1ac826340ebc98a`),
  with no new VPN start or TUN. `release-gate.baBdrR/`.
- **Final state:** `final-log.RlUxgm/` confirms configuration ready, no VPN
  Extension PID and no `vpn-tun`. New emulator and QEMU are left available.

Device tooling boundaries: other work later occupied old targets 15557/5558;
foreground-mismatched reruns there are not counted. No shared HDC server was
reset. New QEMU uses isolated HDC server 8723. Its HDC file-task hang persisted
even with matching client/server/guest versions, so version mismatch is not
claimed as the proven root cause. The exact signed HAP/probe/profile fixtures
were delivered through a [read-only QEMU FAT disk](https://www.qemu.org/docs/master/system/images.html#virtual-fat-disk-images)
and the guest HAP hash was checked before installation. Layout JSON was read
over working HDC shell and screenshots captured through QMP. This is a test
transport workaround, not an application fallback. UI visual acceptance is
based on the new DevEco emulator; QEMU is the VPN lifecycle/data-path target.

### Historical v5 acceptance (2026-09-11–12)

At the v5 checkpoint, root verification covered **406 Rust tests**:
Core **150**, Model **3**, Profile **76 + 1 integration**, UI **138**, VPN **38**.
The full workspace command passed Core and then stopped at one obsolete
source-string UI assertion about the removed proc-based recovery helper.
Updating only that assertion to enforce exact lease and OS-stop-fence behavior
was followed by a complete non-Core rerun: **256/256 passed**. These are combined
final-target results, not a claim that the earlier workspace invocation exited
successfully. Core source and its 150 passing tests were unchanged by this
test-only correction. Platform/ArkTS behavior passed **33/33**. Formatting and
diff checks passed; Clippy completed with advisory warnings.

The v5 unsigned HAP and two public-development-signed packages share native
payload SHA-256 `9d00adbcea70a59e7dd686ff96d3870409319a7e5a15afa82eb659ab32233703`.
Both signed packages passed signature verification. Package hashes:

- unsigned: `2052d086f5334528f9a5d9898ad9e7894bb68bcc8bc5567990460eb070978e42`
- independent emulator: `c230d7cde4c49415c93cc03a9308a9de370fae057ba44aec9664439a5fbece32`
- QEMU 5558: `d6baa32e020b9f37b31be9d3f41f46e259f1f9d59cabc8392425ee217183d6ab`

Final 15557 acceptance passed **14/14 rendered pages** and the Settings/About
layout/navigation/version suite. Root reviewed the actual dashboard image and
a separate sol/xhigh agent visually reviewed all 14 pages of the identical
frozen UI implementation in v3. There were no blocking layout findings. Final
evidence: `smoke-logs/emulator-state-independent/v5-pages/` and `v5-settings/`.

Final QEMU evidence is under `smoke-logs/qemu-state-audit.AMa5eh/v5/`:

- Start `1789140831839859449-1` (UI 1676, Extension 4752) completed at
  23:33:52.669. Ordinary UID `20010041` echoed 20 bytes from the LAN endpoint
  `192.168.3.28:62064` with local address `172.19.0.1`; TUN counters increased.
  Evidence: `start.MU3sqD/`, `verify.TB6VCe/`.
- **Complete Stop passed**, not merely a disconnected projection: request at
  23:38:23.631, native-join/connection-destroy ACK at .682, OS `onDestroy` at
  .699, exact-attempt stop confirmation at .700. Extension PID and TUN were
  absent afterward. Evidence: `stop.rd9FMm/`. An earlier click was blocked by
  a system notification-permission dialog and is explicitly excluded; the
  assertion did not accept it as stop completion. After granting permission,
  the platform logged `notification permission enabled`.
- Restart produced new attempt `1789141167552833651-2`, and ordinary-UID TUN
  echo passed again. Evidence: `start.P4z0Pk/`, `verify.w9ETsj/`.
- A host DevEco SDK replacement interrupted the first freeze run: primary
  `hdc` disappeared and then exited 137. A serial, exact-PID-guarded CONT
  restored the suspended Extension; a subsequent serial check confirmed it
  was no longer paused. An existing compatible HDC client restored observation
  without resetting the shared server. This interrupted run is not counted as
  timed freeze acceptance. See `sdk-interruption.txt`.

- Bounded freeze passed: SIGSTOP for 25 seconds produced Failed; after CONT
  and seven more seconds the old attempt remained terminal. Explicit recovery
  established a new attempt and ordinary-UID TUN echo passed. Evidence:
  `freeze.gMZc00/`, `recover.35AdJ7/`.
- SIGKILL produced Failed with no Extension/TUN; explicit recovery used a new
  Extension and passed ordinary-UID TUN echo. Evidence: `kill.v5yb1A/`,
  `recover.biQ3I1/`. A target-disconnection retry is not counted as an app pass.
- Cold UI recovery passed after killing UI 1676. The OS also terminated its
  Extension 9183, but preserved the Attached journal for attempt
  `1789142128339744178-5`. New UI 12006 confirmed OS stop and exact-owner
  cleanup at 00:01:12.411 before starting `1789142472411471675-1`; Extension
  12523 connected and ordinary-UID TUN echo passed. Complete Stop subsequently
  confirmed at 00:01:51.314. Evidence: `cold-ui-after-kill.txt`,
  `start.CdVezV/`, `verify.CQ6p3M/`, `stop.M6Kgvd/`. Live-Extension survival
  across UI death is not claimed; held-owner behavior has host regressions.

- Fresh authorization passed in `start-pending.2YOaQ0/`: request at
  00:07:09.431, duplicate request reused the pending owner at 00:07:10.595,
  Allow after 00:08:10, exact owner bound at 00:08:11.451, TUN created once and
  start completed at 00:08:12.003. Attempt `1789142829431957679-1` passed
  ordinary-UID echo (`verify.psJHEC/`) and complete Stop (`stop.0aKCol/`).
- The preceding authorization run encountered a device layout-capture timeout
  and exceeded the 120-second startup deadline. Allowing afterward delivered
  a terminal/stale Want which was rejected; no TUN was created. It is not a
  successful first connection. Evidence: `start-pending.dnMpT6/`,
  `authorization-expired-before-allow.log`, `authorization-expired-late-want.log`.

The subsequently found release automation gate and cleanup-publication failure
were fixed in v6/v7; final v7 results above supersede this historical checkpoint.

Historical v2 host verification on 2026-09-10:

- `cargo test --workspace --locked`: **374 passed, 0 failed** (38 test/doc-test
  targets). Breakdown: Core 128, Model 3, Profile 76 + 1 integration, UI 128,
  VPN 38. The run requires local TCP listener access outside the filesystem/
  network sandbox; the initial restricted run was denied socket binding.
- `node --test scripts/test-vpn-platform.mjs`: **30 passed, 0 failed**,
  including real application ArkTS submit-once/typed-pending behavior and ScanKit
  numeric cancellation versus genuine errors containing the word `cancel`.
  The final run includes never-resolving system-start Promises, exact attached/
  terminal delivery acknowledgements, stale delivery rejection and confirmed
  OS-stop recovery. Full Rust output: `smoke-logs/state-audit-workspace-tests-final.log`.
- `cargo check -p paws_ui --lib --locked`, `cargo fmt --all --check` and
  `git diff --check`: passed.
- `cargo clippy --workspace --all-targets --locked`: completed successfully,
  with advisory warnings (including pre-existing derivable defaults/argument
  counts/test borrows and the prepared-import enum size). This is not a claim
  of `-D warnings` cleanliness: that stricter experiment stops at two baseline
  `paws_model` manual `Default` implementations. The reported test-only
  mutex-across-await site explicitly drops the guard before awaiting; no
  production lock-across-await warning was reported.
- At this stage device acceptance was still in progress; these historical host
  results alone did not close UI or VPN data-path requirements.

Intermediate v3 verification on 2026-09-11:

- Finalized UI/Profile/Model/VPN targets: `cargo test --workspace --exclude
  paws_core --locked` passed **256 tests**, 37 test/doc-test targets. Core is
  separately reopened for the sandbox identity finding above. The first Core
  run passed 145/146; its remaining test omitted the newly required explicit
  OS-stop fence confirmation. Correcting that test prerequisite passed its
  targeted rerun; this is not recorded as a complete workspace pass.
- Actual ArkTS release packaging passed after correcting a catch/throw syntax
  constraint that the TypeScript behavioral harness does not enforce.
  Platform behavioral regressions passed **32/32**. Clippy completed with
  advisory warnings, not `-D warnings` cleanliness.
- Independent 15557 UI rerun passed all **14** pages with v3; evidence is in
  `smoke-logs/emulator-state-independent/v3-pages/`.
- QEMU v2-to-v3 `install -r` terminated both active legacy PIDs (UI 2701,
  Extension 14825) and removed `vpn-tun` **before** launching the new Ability.
  The baseline had transferred an ordinary-UID echo and had no owner journal.
  This establishes package-update migration, not hot-swapping a live legacy
  Extension. Evidence: `v3/upgrade-before-new-ability.txt`.
- v3 real VPN startup failed the sandbox identity check. The failed UID probe
  echoed over local `10.0.2.15`, and the assertion correctly rejected it as
  non-TUN traffic. It is **not** VPN data-path acceptance. Failure evidence:
  `v3/identity-start-failure.log`, `identity-start-processes.txt`,
  `identity-proc-mount.txt` and `verify.JVcwdN/` under the QEMU audit directory.

### Device acceptance work log

Final UI acceptance uses a newly created, independent DevEco instance
`Paws_State_Audit`, target `127.0.0.1:15557`, under
`smoke-logs/emulator-state-independent/`. The shared 5555 simulator was found
to be used by another application test and is no longer used for UI actions.
The new instance uses HarmonyOS 6.1.1 API 24 image / software 6.1.0.126,
1320 × 2856 at 560 dpi, with its own writable images and debug port.

On 15557, a deliberately delayed 20-second HTTP profile request was cancelled
through the rendered dialog. After the server completed its delayed response,
the UI still showed no profiles and `profiles.json` still contained zero
profiles. A subsequent ordinary HTTP import succeeded through the actual URL
dialog and populated `direct.yaml` with its proxy group and rule fixtures.

On 2026-09-11, `harmony-state-ui-smoke.py` passed all **14** actual page-title
and no-rendered-panic assertions against the v2 package: dashboard, proxies,
profiles, traffic, settings, appearance, network, converter, requests,
connections, resources, logs, about and privacy. Root also visually inspected
the final rendered pages. Layout JSON and screenshots are retained in
`smoke-logs/emulator-state-independent/final-pages/`. The script distinguishes
an actionable Settings Button from a visible same-name section heading; a
heading-only match was an automation defect, not an application failure.

Additional v2-package interactions on 15557:

- Enabled recording and opened a nonempty history list: the daily archive and
  active-writer explanation rendered without a detached-tree context panic.
  After generating import/save events, stopped recording, backed up the 2,217-byte
  test archive, opened its delete confirmation from the virtual row, and
  confirmed deletion. The UI reported success and zero archives without panic;
  the test archive remains recoverable from the acceptance directory.
- Entered an unsaved port `17892`, then activated `Revision-Switch` via a real
  checked DebugAutomation import. The existing Network page retained its draft,
  displayed the conflict notice, and disabled Save. The active index identified
  the new profile and its YAML contained no `17892` override. Explicitly choosing
  “放弃编辑并载入当前配置” removed the conflict and loaded the current `7890` value.
  A subsequent fresh edit/save to `17892` succeeded, returned the explicit
  next-connection acknowledgement, disabled Save again, and persisted
  `paws.mixed-port: 17892` in that newly active profile.
  Evidence: `smoke-logs/emulator-state-independent/interactive-final/`.
- Scrolled the Resources virtual list to its Rules section and opened the
  manual-rule editor through the row's Add action. The dialog rendered its
  default draft/preview with Save disabled for the empty matcher, without panic.
  Closing it returned to the unchanged zero-rule list.
- Re-ran `harmony-settings-ui-smoke.sh` with `INSTALL_HAP=0` against the final
  15557 installation: Settings grouping/alignment, secondary-page navigation,
  About labels/revision truncation and repository-link baseline checks passed.
  Evidence: `smoke-logs/emulator-state-independent/settings-final/`.

DevEco target `127.0.0.1:5555` (1320 × 2856) has exercised the rendered UI:

- Imported `Audit-UI` using the URL/name dialog and an actual local HTTP
  download; closing the visible soft keyboard preserved both input drafts.
- Opened its YAML editor, validated the original content, introduced invalid
  syntax, observed the parser error, and rolled the draft back without
  persisting the invalid content.
- Changed the network port to `17891` and saved it; the profile YAML contains
  the resulting `paws.mixed-port` override. Unsupported system proxy/bypass
  settings are visibly read-only with explanations.
- Edited the converter source draft and left the page; the app persisted it.
  The original converter file was then restored from the pre-install backup
  and verified byte-for-byte with `cmp`.
- Switched to dark appearance and observed both the rendered dark palette and
  the successful platform acknowledgement. Switched to English and observed
  the translated page; restored both language and appearance to System and
  checked the resulting light Chinese page.
- `harmony-settings-ui-smoke.sh` passed its layout, child navigation and
  version assertions. Its outdated root group labels were updated to the
  existing Interface/Network/Tools layout (not a production layout change).

Evidence is under `smoke-logs/emulator-state-audit/`, including layout JSON,
screenshots and the isolated test profile. Original app data was backed up
before replacing the emulator-only installation to use a public test signing
identity; it is not release signing material.

QEMU target `127.0.0.1:5558` has established a real `vpn-tun` and transferred an
echo payload under an ordinary application UID with source `172.19.0.1` and
increasing TUN packet/byte counters. The host LAN endpoint is used because the
QEMU host gateway `10.0.2.2` can be reached through a directly connected route.
The initial run exposed the lifecycle issue described below; final-package
acceptance was subsequently repeated independently.

v2 package, 2026-09-11 19:20–19:21 CST (superseded; stop acceptance failed):

- Ordinary application UID `20010041` returned the exact 20-byte payload
  `paws-qemu-final-echo` from `192.168.3.28:62064`, using local source
  `172.19.0.1`. TUN RX increased from 0 packets / 0 bytes to 5 / 232;
  TX increased from 7 / 448 to 12 / 688. The same UI/VPN PIDs, 2198/2209,
  appear in both counter captures.
- The initial UI-only Stop check returned in about 2.2 seconds. The stop request was logged at
  19:21:14.408 and extension `onDestroy` at 19:21:14.432. The resulting UI
  was disconnected, the extension PID was absent, and `ifconfig vpn-tun`
  returned `No such device`. This start used existing system authorization;
  it is not evidence of a fresh authorization prompt or completed stop transaction.
  Subsequent root review **revoked the stop pass**: at 19:23:14.444, the
  operation failed with `did not stop before the cleanup deadline`, and the
  queued restart failed with `previous platform VPN connection cleanup is still
  pending`. OS resource disappearance and a disconnected UI are insufficient
  evidence of the cleanup receipt. Session cleanup and UI operation-pending
  ownership have been reopened; see `root-restart-timeout-PawsVpn.log`.
- The same attempt received duplicate Wants, logged as ignored before a single
  TUN creation. The QEMU notification permission was disabled and was explicitly
  logged as unavailable; this run does not claim successful system-notification
  presentation. Granted/denied notification branches retain host regression
  coverage independently of successful VPN transport.
- Root reviewed `non-root-tunnel-echo.txt`, `tunnel-before-echo-network.txt`,
  `tunnel-after-echo-network.txt`, and `stopped{,-network,-PawsVpn}` evidence
  under `smoke-logs/qemu-state-audit.AMa5eh/v2/final/`.

Final-package attempts initially encountered endpoint and guest-availability problems: the
single-client echo fixture could block on an idle connection and was replaced
with independently served connections plus EOF handling. Host loopback and LAN
payload self-checks subsequently passed. During extended approval/idle intervals,
the guest also lost HDC and display output while QMP remained responsive with
`running=true`; host sampling found all four HVF vCPUs waiting, not CPU saturation.
Wake attempts did not restore the guest. Recovery is limited to the isolated
5558 VM, with `qemu-no-hdc.png` and `qemu-hang-host-sample.txt` preserved. These
observations do not establish the cause of the whole-guest inactivity. Failed
probes and guest downtime are not counted as successful VPN acceptance.
Cold-starting the same isolated images restored HDC. After checking guest
`power-shell help`, the test used `power-shell timeout -o 86400000` and `wakeup`
to avoid automatic screen-off during acceptance, following the
[OpenHarmony power-shell documentation](https://raw.githubusercontent.com/openharmony/docs/master/en/application-dev/tools/power-shell.md).
This is a test-device setting, not a production application workaround.

QEMU also exposed a device-only stop blocker: the system start API may leave
its Promise unresolved even after the matching extension is attached and
connected. Waiting only for that Promise prevents stop from progressing.
The earlier v2 package used an exact-attempt attachment/delivery barrier. The
v3 correction durably fences Pending owners as Stopping before a late Want can
attach, so an unresolved system-start Promise is no longer a stop barrier.
Attached owners first receive cooperative stop; exceptional cleanup requires a
confirmed OS stop and exact owner-lease release in v5, not a fixed quiet-period
delay or sandbox-hidden peer proc observation.
Host regressions cover both attached and terminal late-Want delivery; actual
lifecycle acceptance is recorded separately below.

The rendered resource page also exposed a missing `LocalRuleEditors` context
inside the native virtual list's detached Dioxus tree. Its screenshot contained
`Encountered panic` even though screenshot capture itself succeeded. The final
implementation passes typed Signals/EventHandlers/palettes explicitly into
detached rows. Archive/proxy pending actions use native loading indicators with
explicit colors rather than a Theme-context-dependent Spinner. Real Dioxus
detached-tree regression coverage and final device interactions now pass. The new
`scripts/harmony-state-ui-smoke.py` asserts actual page titles and rejects
rendered panic text; it uses observed header navigation and distinguishes the
About entry from the similarly named group heading. The pre-existing local
page capture helper was left unchanged and is not treated as a passing test.
The assertion-based script was run against the pre-fix device package and
correctly failed on the resource-page panic after ten successful page checks.

Final review additionally found strict YAML private-option parsing, legacy NAPI
mutation exposure, rollback-secondary-failure coverage and observable log I/O
gaps. The final implementation validates private options before sanitization,
removes unused unchecked mutation exports, commits automation import/activation
as one checked transaction, preserves both primary and rollback errors, and
projects recording I/O errors visibly to the Logs page. Fault-injection and
malformed-input regressions were included in the historical 374-test workspace
run and retained in the final 406-test coverage.

### Acceptance boundaries

Historical v2 device packages share the same unsigned HAP and native payload;
only the public test signing profiles differ. SHA-256 provenance:

- unsigned HAP: `2dade3e6da8e3c44bb6c11bfd2c3069a0f208547e05e948f8163e73e03c3cf24`
- packaged `libs/arm64-v8a/libpaws_ui.so`: `d90963b8a4a0b1860522bb6910951678579dbd702c0777a644f25283caf457c0`
- `paws-state-audit-emulator-v2-final.hap`: `d6d8fd2940282a47aacf2d280ee06fb32c471b21fb524a19c594eb87fdd97739`
- `paws-state-audit-qemu-5558-v2-final.hap`: `25c1ae42064ef47e48571f1deed8b0d7d8a6815c2cf7f245e57fd9ec731bbe9c`

The signed packages are retained under
`smoke-logs/qemu-state-audit.AMa5eh/v2/final/`, excluded from Git together with
device images and signing files. They are acceptance builds, not release-signed
distribution artifacts.

- Dioxus Signals own reactive UI projections, not the VPN service. The
  process-local core remains the owner of durable configuration transactions
  and native runtime resources; the two platform processes exchange explicit
  session-scoped IPC events.
- Dialog drafts and queries belong to their page/component. Committed
  mutations finish their transaction even if the page goes away, but their
  callbacks cannot update a disposed page or replaced root.
- `allowBypass` and `systemProxy` are not supported by the pinned platform VPN
  API. Existing true values remain visible and require an explicit clear;
  new unsupported writes/start requests are rejected, never silently ignored.
- DevEco emulator acceptance covers actual rendered UI and interactions.
  It does **not** establish a VPN. QEMU acceptance must separately prove a
  real `vpn-tun`, application-UID payload transfer, cleanup and failure/recovery.
- The platform TypeScript harness executes application logic with mocked OS
  boundaries. It cannot establish OS service teardown or routing correctness.
- The pinned ability bridge clamps individual calls to 60 seconds. VPN
  operations therefore use one submission and an Ability-instance-scoped
  operation ID, followed by 45-second observation slices (55-second transport
  deadline). The plugin retains at most 16 operations and never evicts active
  work. Losing an acknowledgement or exhausting the business wait budget
  yields a typed `Unconfirmed` result, not a retry, cancellation, or claim of
  VPN failure. This small platform transport protocol is separate from the
  removed universal UI action dispatcher.
- `verify-local-protocols.sh` validates all local protocol fixture/server
  modes. The core echo tests and the QEMU UID probe provide actual data-path
  evidence; fixture startup alone is not a successful VPN test.

The pre-refactor workspace run passed 84 core tests, 3 model tests, and 65
profile tests. It then stopped on two pre-existing source-string assertions in
`diagnostics_ui_contract` (a Textarea count and a helper call spelling).

The platform harness runs the actual application `.ets` modules with the SDK's
TypeScript transpiler and controlled platform/NAPI boundaries:

```sh
node --test scripts/test-vpn-platform.mjs
```

Set `PAWS_TYPESCRIPT_PATH` to the SDK TypeScript package if it is not discoverable
through `OHOS_NDK_HOME`, `DEVECO_SDK_HOME`, or the standard macOS DevEco location.
The initial implementation fails the malformed JSON, invalid CIDR, stale Want,
and deferred create/destroy ordering cases; default configuration and Want FD
transfer pass. This harness proves application ordering and validation behavior,
not platform service behavior or actual network routing.

# Paws UI contract

The pinned NSH Design Guides and Gallery are the reference for Paws. UI code
uses public NSH controls; Core commands, receipts and domain ownership remain
outside presentation. See [the three-layer architecture](nsh-three-layer-architecture.md).

## Foundations

- `theme.json` defines light/dark semantic colors and radius tokens.
  `src/theme.ts` maps them to application roles. Ordinary sections have a flat
  surface and an external heading; shadows belong to raised controls and overlays.
- Phone controls use 40-point standard buttons/fields, 32-point small actions
  and 36-point icon buttons. Action labels use 13/12-point text and tighter
  padding. Native `hitSlop` retains 44-point button press areas without enlarging
  the painted control. Text rows grow with font scale. Desktop controls retain their scale;
  narrow desktop windows retain touch-sized targets. Long values wrap or have
  a complete details view, and comparable numbers
  use tabular figures.
- The wide shell has a scrolling sidebar and a bounded workspace. Narrow windows
  use the bottom navigation for root destinations and native Back for details.
  Resizing retains the current route and its drafts.

## Controls and surfaces

- Primary buttons commit the focused task. Ordinary commands use default,
  outline or ghost buttons. Busy actions keep their label and show a spinner.
- Switches commit immediate settings. Checkboxes collect independent draft
  choices for an explicit Save. Native radio groups select immediate preferences.
- Selects use native Menu radio items. Object commands use the same command set
  for the visible menu and native right click / long press. Phone menus are
  sheets; PC menus are anchored popups. Commands that open another surface wait
  for the menu's native close lifecycle.
- Consequential actions use `AlertDialog.show`, name their scope, and label the
  result explicitly. A canceled dialog never submits a command. Async results
  are fenced to the owning route and lifetime.
- Short forms have visible labels, a scrolling body and a fixed footer. Native
  Scroll owns focused-input visibility; dialogs and the retained YAML editor
  also account for the keyboard around their fixed content.
- External destinations use native Link semantics and the existing platform
  command boundary. Copy and other application commands remain buttons.

## Page behavior

| Area | Presentation and interaction |
| --- | --- |
| Home and proxies | Connection focal point, quiet metadata, continuous group/member list, stable selection and explicit disclosure |
| Subscriptions | Search above a continuous list, current/update/usage metadata, native object menus, short import/edit forms |
| Traffic | Comparable usage figures, distinct upload/download chart series, a true zero baseline, connection and DNS drill-down |
| Connections and requests | Virtual lists, network/status filters, full destination and route details, per-object commands |
| Resources | Compact provider/rule rows, details on demand, explicit ordering and scoped revision-safe commands |
| Logs | Stable row identities, visible recording/filter state, full-message details and archive menus |
| Settings and converter | Grouped draft forms, saved choices, one explicit commitment for each task, local validation and failure feedback |
| Diagnostics and backups | Separate current state, probes/policy and local/remote workflows; specific restoration confirmations |
| Appearance, About and privacy | Native preferences, readable full version/data text and external links |
| YAML editor | Retained native Editor, find/wrap/validate/save, exact drafts and native discard decisions |

## Motion and keyboard

`src/motion.ts` owns the Gallery springs: press 520/34, controls 1218/70 and
movement 504/38. Press feedback, selection indicators, switch thumbs and
chevrons use native animation values. Explicit disclosure uses a short layout
transition; recurring telemetry never schedules a layout or entrance animation.
Menus, dialogs and navigation use their native lifecycle. Notifications and
floating-button docking honor reduced motion, including when it changes.

Desktop shortcuts are Mod+1–4 for root destinations, Mod+, for appearance and
Mod+R for refresh. The active YAML editor owns Mod+S (save), Mod+F (find) and
Mod+Shift+R (reload with its discard guard). Native menus, radio groups and
dialogs own their keyboard traversal, focus and Escape behavior.

## Verification boundary

Acceptance tests exercise actual production views and CoreClient with public
synthetic fixtures. Geometry tests cover narrow and desktop widths, both themes,
long content, sibling containment and end-of-scroll access. Platform-specific
tests set the desktop idiom explicitly; width alone is insufficient.

Headless results do not prove OHOS pixels, IME, frame cadence or real traffic.
The release candidate is installed with transfer hash verification, regressed on
the physical Phone first, and then on the local MateBook PC emulator. Personal
profiles are preserved and never used for synthetic YAML edits or VPN starts.

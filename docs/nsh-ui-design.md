# Paws UI contract

The pinned NSH Design Guides and Gallery are the reference for Paws. UI code
uses public NSH controls; Core commands, receipts and domain ownership remain
outside presentation. See [the three-layer architecture](nsh-three-layer-architecture.md).

## Foundations

- `theme.json` defines light/dark semantic colors and radius tokens.
  `src/theme.ts` maps them to application roles. Ordinary sections have a flat
  surface and an external heading; shadows belong to raised controls and overlays.
- Control sizes follow the NSH Gallery tier for the input method, selected by
  `platform.desktop` through `control` and `typography` in `src/theme.ts`.
  Phone and Pad use the touch tier; PC uses the pointer tier. Pad stays on the
  touch tier even with a keyboard.

  | Control | Touch (Phone, Pad) | Pointer (PC) |
  | --- | --- | --- |
  | Button regular / small | 40 / 36, 15 / 14-point label | 28 / 24, 13 / 12-point label |
  | Icon button | 40, icon 18 | 28, icon 15 |
  | Text field | 46, 15-point text | 28, 13-point text |
  | Select, Picker | 44 | 32 |
  | Segmented control regular / small | 36 / 32 | 30 / 26 |
  | Switch | 42 x 24, knob 20 | 28 x 16, knob 12 |
  | Checkbox, radio | 22 | 16 |
  | List row (least height) | 48 | 40 |
  | Section surface radius | 16 | 10 |
  | Body, row label / note | 15 / 13 | 14 / 12 |
  | Menu row, sheet row | 46, 16-point text | 26, 13-point text |

  Section surfaces are flat: a 1-point edge and no shadow. Badges are 26 points
  tall with 12-point text on both tiers. The compact filter picker is 32 points
  on both tiers so it pairs with the small segmented control; Gallery has no
  touch size for it. Touch `hitSlop` retains 44-point press areas without
  enlarging the painted control. Header titles use 17-point text. Text rows grow
  with font scale. Long values wrap or have a complete details view, and
  comparable numbers use tabular figures.
- Desktop always uses its own workspace: a 232-point scrolling sidebar (28-point
  items), or a 64-point icon rail (36-point tiles) in a narrow window, with
  tooltips and keyboard focus. The brand block is 40 points tall without a
  divider; the content toolbar is 54 points, and the Phone header is 52 points. Root destinations are direct switches; Back belongs to nested
  tasks. The mobile Settings hub and duplicate request-history link are absent
  from the desktop sidebar.
- Overview renders all policy groups directly without truncation or collapse on
  every platform; wide viewports separate the status column (connection hero,
  node summary and speed) and the full policy-group column. Desktop nodes use a
  group browser and member pane. Connections and request history share one
  workspace with count-badged tabs, compact protocol/sort/status filters, Phone
  bottom-sheet details, Pad master-detail, and PC `DataTable` (40-point rows, 32-point header)
  plus a closable inspector. Object commands use anchored menus and right click;
  short forms use Phone bottom sheets or centered Pad/PC dialogs.
- Phone and narrow Pad use bottom navigation for root destinations and native
  Back for details. Pad uses the sidebar from 840 logical points, independent of
  orientation. `src/layout.ts` subtracts its 64/232-point sidebar, page cap,
  padding and gutter before admitting 240/320-point collection/detail minima.
  Font growth increases those minima; insufficient space uses two pages.
  Root switches retain each workspace stack, query, object and scroll anchor;
  Back only pops the current workspace. Resize preserves mounted drafts.

## Controls and surfaces

- Primary buttons commit the focused task; active VPN stop actions use solid red
  (`stop` token) with white foreground and icon/spinner. Ordinary commands use
  default, outline or ghost buttons. Busy actions keep their label and show a
  spinner.
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
| Home and proxies | Connection focal point with solid red stop action, all policy groups expanded on Home, group/member browsing, stable selection |
| Subscriptions | Search above a continuous list, current/update/usage metadata, native object menus, Gallery `ChoiceTile` local/QR import + URL form |
| Traffic | Comparable usage figures, distinct upload/download chart series, a true zero baseline, connection and DNS drill-down |
| Connections and requests | Unified workspace with active/history tabs, compact filters, Phone bottom sheet, Pad master-detail, PC `DataTable` + inspector |
| Resources | Compact provider/rule rows, details on demand, explicit ordering and scoped revision-safe commands |
| Logs | Pause/resume & clear header actions, current/archive tabs, 5-level tabs, `13.5 pt` monospace rows, follow pill, sheet/inspector details |
| Settings and converter | VPN/DNS/network drafts save independently; converter auto-saves on departure, marks generated output stale after edits, and preserves failed drafts |
| Diagnostics and backups | Separate current state, probes/policy and local/remote workflows; specific restoration confirmations |
| Appearance, About and privacy | Native preferences, readable full version/data text and external links |
| YAML editor | Retained native Editor, find/wrap/validate/save, exact drafts and native discard decisions |

## Motion and keyboard

`src/motion.ts` owns the Gallery springs: press 520/34, controls 1218/70 and
movement 504/38. Press feedback, selection indicators, switch thumbs and
chevrons use native animation values. Explicit disclosure uses a short layout
transition; recurring telemetry never schedules a layout or entrance animation.
Virtualized group/member lists keep recycled rows opaque: expansion changes the
row data without a global create/delete fade. The chevron and row background
still animate on the native thread.
Menus, dialogs and navigation use their native lifecycle. Notifications and
floating-button docking honor reduced motion, including when it changes.

Desktop shortcuts are Mod+1–4 for Overview, Subscriptions, Proxy nodes and Traffic,
Mod+, for appearance and
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

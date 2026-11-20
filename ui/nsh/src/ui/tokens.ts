import {
  token,
  type Tokens,
  type ViewStyle,
  colorScheme,
  platform,
  fontScale,
  reduceMotion,
  safeArea,
  windowSize,
} from "nsh";
import finishes from "../finishes.json";

// NSH Gallery dual-tier design tokens:
// - Touch tier (Phone & Pad): comfortable finger targets (40pt buttons, 46pt fields, 46pt menu rows, 22pt checks/radios).
//   Pad remains on the touch tier even with a keyboard attached, so `platform.desktop` alone selects the tier.
// - Pointer tier (PC): crisp desktop density (28/24pt buttons, 32pt fields & selects, 24pt small segments & compact pickers,
//   26pt menu rows, 16pt checks/radios).
// All colors derive from nsh.json's theme.json so built-in Editor parts stay in sync.
export const density = {
  get compact() {
    return false;
  },
  font(size: number) {
    return size;
  },
  line(size: number) {
    return size;
  },
  space(size: number) {
    return size;
  },
  icon(size: number) {
    return size;
  },
};

export const spacing = {
  get xxs() {
    return density.space(4);
  },
  get xs() {
    return density.space(4);
  },
  get sm() {
    return density.space(8);
  },
  get md() {
    return density.space(12);
  },
  get lg() {
    return density.space(16);
  },
  get xl() {
    return density.space(24);
  },
  get xxl() {
    return density.space(24);
  },
  get section() {
    return density.space(24);
  },
  get region() {
    return density.space(32);
  },
};

const pointer = () => platform.desktop;

export const radius = {
  /** Inline chips / small badges / tooltip capsules: 5 pointer, 6 touch. */
  get sm() {
    return pointer() ? 5 : token("radius-small");
  },
  /** Interactive controls (buttons, fields, selects): 7 pointer, 10 touch. */
  get md() {
    return pointer() ? 7 : token("radius-control");
  },
  /** Popups, menus, wells: 10 pointer, 12 touch. */
  get lg() {
    return pointer() ? 10 : token("radius");
  },
  /** Section surfaces & cards: 10 pointer, 16 touch. */
  get xl() {
    return pointer() ? 10 : token("radius-card");
  },
  /** Dialogs: 16 both tiers. */
  get xxl() {
    return token("radius-dialog");
  },
  /** Bottom sheet top corners on touch: 20. */
  sheet: 20,
  full: 999,
};

export const typography = {
  get xs() {
    return density.font(12);
  },
  /** Secondary text and row details: 12 pointer, 13 touch. */
  get note() {
    return density.font(pointer() ? 12 : 13);
  },
  /** Body text and row labels: 14 pointer, 15 touch. */
  get sm() {
    return density.font(pointer() ? 14 : 15);
  },
  /** Section titles: 13 pointer, 15 touch. */
  get section() {
    return density.font(pointer() ? 13 : 15);
  },
  get md() {
    return density.font(17);
  },
  get lg() {
    return density.font(18);
  },
  get xl() {
    return density.font(20);
  },
  get xxl() {
    return density.font(24);
  },
};

export const control = {
  get compact() {
    return !pointer();
  },
  /** Regular button: 28 pointer, 40 touch. */
  get height() {
    return pointer() ? 28 : 40;
  },
  /** Small button: 24 pointer, 36 touch. */
  get small() {
    return pointer() ? 24 : 36;
  },
  /** Large button: 34 pointer, 48 touch. */
  get large() {
    return pointer() ? 34 : 48;
  },
  /** Icon button: 28 pointer, 40 touch. */
  get icon() {
    return pointer() ? 28 : 40;
  },
  /** Text field & search field (Gallery fieldSize): 32 pointer, 46 touch. */
  get field() {
    return pointer() ? 32 : 46;
  },
  /** Select trigger (level with text field on PC): 32 pointer, 44 touch. */
  get select() {
    return pointer() ? 32 : 44;
  },
  /** Segmented control: 30 pointer, 36 touch. */
  get segment() {
    return pointer() ? 30 : 36;
  },
  /** Small segmented control (level with small button on PC): 24 pointer, 32 touch. */
  get segmentSmall() {
    return pointer() ? 24 : 32;
  },
  /** Compact filter picker, level with small segmented control: 24 pointer, 32 touch. */
  get compactHeight() {
    return pointer() ? 24 : 32;
  },
  /** Checkbox and radio box: 16 pointer, 22 touch. */
  get box() {
    return pointer() ? 16 : 22;
  },
  /** Menu item row (Gallery menuItemSize): 26 pointer, 46 touch. */
  get menuRow() {
    return pointer() ? 26 : 46;
  },
  /** Menu item corner radius (concentric with popupSurface): 6 pointer, 10 touch. */
  get menuRadius() {
    return pointer() ? 6 : 10;
  },
  /** Vertical inset for list cells: 6 pointer, 12 touch. */
  get rowPadding() {
    return pointer() ? 6 : 12;
  },
  /** Least height of a list row: 40 pointer, 56 touch. */
  get row() {
    return Math.max(
      pointer() ? 40 : 56,
      density.line(20) * Math.max(1, fontScale()) + (pointer() ? 16 : 24),
    );
  },
  hitSlop(size: number) {
    return !pointer() ? Math.max(0, (44 - size) / 2) : 0;
  },
};

export const chrome = {
  header: 52,
  /** Unified seamless desktop/wide toolbar height (matches nsh.json titleBar.height). */
  toolbar: 48,
  bottom: 52,
};

const names = {
  background: "background",
  panel: "card",
  qrCanvas: "qr-canvas",
  text: "foreground",
  muted: "muted-foreground",
  faint: "faint",
  border: "border",
  accent: "brand",
  primaryText: "on-brand",
  brandFill: "brand-fill",
  stopFill: "stop-fill",
  onStop: "on-stop",
  accentFill: "brand-tint",
  soft: "muted",
  hover: "hover",
  field: "field",
  fieldBorder: "input",
  raised: "raised",
  track: "track",
  knob: "knob",
  code: "code",
  info: "info",
  infoFill: "info-tint",
  danger: "destructive",
  dangerText: "destructive-foreground",
  dangerFill: "destructive-tint",
  success: "up",
  successFill: "up-tint",
  warning: "warn",
  warningFill: "warn-tint",
  ring: "ring",
  sidebar: "sidebar",
  sidebarAccent: "sidebar-accent",
  sidebarBorder: "sidebar-border",
  inset: "inset",
  line: "line",
  popover: "popover",
  scrim: "overlay-scrim",
  chartDownload: "chart-1",
  chartUpload: "chart-2",
} satisfies Record<string, keyof Tokens>;

const colors = {} as Record<keyof typeof names, string>;
for (const name of Object.keys(names) as (keyof typeof names)[])
  Object.defineProperty(colors, name, {
    enumerable: true,
    get: () => token(names[name]),
  });

export function palette() {
  return colors;
}

export function surfaceFinish(kind: keyof typeof finishes.light): ViewStyle {
  return (colorScheme() === "dark" ? finishes.dark : finishes.light)[kind];
}

/** Outer keyboard focus ring (`focusVisibleStyle`): never inset into borders/text.
 * Pass `0` for rows clipped inside an `overflow: hidden` card so the ring stays visible. */
export function focusRing(offset: number | boolean = 1): ViewStyle {
  const outlineOffset =
    typeof offset === "boolean" ? (offset ? 0 : 2) : offset;
  return {
    outlineStyle: "solid",
    outlineWidth: 2,
    outlineColor: colors.ring,
    outlineOffset,
  };
}

/** Whether menus/dropdowns should present as an anchored floating popover rather
 * than a bottom sheet (covers desktop OSes plus Pad / HarmonyOS 2-in-1 wide windows). */
export function useWidePopover(): boolean {
  return !platform.desktop && windowSize().width >= 768;
}

/** Shared popup surface for Menu, ContextMenu, Popover, Select, and CompactPicker. */
export function popupSurface(
  presentation?: "anchored" | "sheet",
  maxWidth?: number,
): ViewStyle {
  // Matches nsh/base's `as_sheet()` presentation when rendered inside `<Menu>`.
  // On wide non-desktop viewports (>= 768pt), `menu.tsx` switches to `<Popover>`
  // and passes `"anchored"`, keeping a crisp floating card next to the trigger.
  const menuAsSheet =
    presentation === "sheet" ||
    (presentation !== "anchored" &&
      !platform.desktop &&
      !(
        platform.os === "ios" &&
        platform.idiom === "pad" &&
        windowSize().width >= 768
      ));
  return {
    minWidth: 220,
    maxWidth: menuAsSheet ? undefined : (maxWidth ?? 340),
    padding: spacing.xxs,
    borderRadius: radius.lg,
    borderCurve: "continuous",
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.popover,
    ...(menuAsSheet
      ? {
          borderTopLeftRadius: radius.sheet,
          borderTopRightRadius: radius.sheet,
          borderBottomLeftRadius: 0,
          borderBottomRightRadius: 0,
          borderBottomWidth: 0,
          paddingBottom: Math.max(spacing.sm, safeArea().bottom),
        }
      : {}),
    ...surfaceFinish("menu"),
    transitionProperty: "opacity, transform",
    transitionDuration: reduceMotion() ? 0 : 150,
    transitionTimingFunction: "cubic-bezier(0.16, 1, 0.3, 1)",
  };
}

export type StatusTone =
  | "neutral"
  | "info"
  | "brand"
  | "success"
  | "warning"
  | "danger";

export function statusColors(tone: StatusTone = "neutral") {
  const text =
    tone === "brand"
      ? colors.accent
      : tone === "info"
        ? colors.info
        : tone === "success"
          ? colors.success
          : tone === "warning"
            ? colors.warning
            : tone === "danger"
              ? colors.danger
              : colors.muted;
  const background = token(
    tone === "brand"
      ? "brand-tint"
      : tone === "info"
        ? "info-tint"
        : tone === "success"
          ? "up-tint"
          : tone === "warning"
            ? "warn-tint"
            : tone === "danger"
              ? "destructive-tint"
              : "muted",
  );
  return { text, background, border: colors.border };
}

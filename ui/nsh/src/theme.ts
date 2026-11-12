import {
  token,
  type Tokens,
  type ViewStyle,
  colorScheme,
  platform,
  fontScale,
  reduceMotion,
} from "nsh";
import finishes from "./finishes.json";

// Phone, Pad, and PC share one unified NSH Gallery component/typography scale;
// only workspace navigation and multi-pane breakpoints adapt to viewport/device.
// All colors come from nsh.json's theme so built-in Editor parts share them.
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
export const radius = {
  sm: 6,
  md: 10,
  lg: 12,
  /** Section surfaces: the theme's card radius on touch, 10 under a pointer. */
  get xl() {
    return platform.desktop ? 10 : token("radius-card");
  },
  get xxl() {
    return token("radius-dialog");
  },
  full: 999,
};
// NSH Gallery sizes by input method: the pointer tier (desktop) is compact,
// the touch tier (Phone and Pad) roomier. Pad stays on the touch tier even
// with a keyboard, so `platform.desktop` alone selects the tier.
const pointer = () => platform.desktop;
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
  /** Text field: 28 pointer, 46 touch. */
  get field() {
    return pointer() ? 28 : 46;
  },
  /** Select and picker trigger: 32 pointer, 44 touch. */
  get select() {
    return pointer() ? 32 : 44;
  },
  /** Segmented control: 30 pointer, 36 touch. */
  get segment() {
    return pointer() ? 30 : 36;
  },
  /** Small segmented control: 26 pointer, 32 touch. */
  get segmentSmall() {
    return pointer() ? 26 : 32;
  },
  /** Compact filter picker, paired with the small segmented control: 32 both tiers. */
  compactHeight: 32,
  /** Checkbox and radio box: 16 pointer, 22 touch. */
  get box() {
    return pointer() ? 16 : 22;
  },
  /** Least height of a list row: 40 pointer, 48 touch. */
  get row() {
    return Math.max(
      pointer() ? 40 : 48,
      density.line(20) * Math.max(1, fontScale()) + 16,
    );
  },
  hitSlop(size: number) {
    return !pointer() ? Math.max(0, (44 - size) / 2) : 0;
  },
};
export const chrome = {
  header: 52,
  toolbar: 54,
  bottom: 64,
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
export function focusRing(inset = true): ViewStyle {
  return {
    outlineStyle: "solid",
    outlineWidth: 2,
    outlineColor: colors.ring,
    outlineOffset: inset ? -2 : 2,
  };
}
export function popupSurface(): ViewStyle {
  return {
    minWidth: 224,
    maxWidth: 480,
    padding: spacing.xxs,
    borderRadius: radius.lg,
    borderCurve: "continuous",
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.popover,
    ...surfaceFinish("menu"),
    transitionProperty: "opacity, transform",
    transitionDuration: reduceMotion() ? 0 : 150,
    transitionTimingFunction: "cubic-bezier(0.16, 1, 0.3, 1)",
  };
}
export type StatusTone = "neutral" | "info" | "brand" | "success" | "warning" | "danger";
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

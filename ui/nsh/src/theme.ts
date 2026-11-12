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
  get xl() {
    return token("radius-card");
  },
  get xxl() {
    return token("radius-dialog");
  },
  full: 999,
};
export const typography = {
  get xs() {
    return density.font(12);
  },
  get note() {
    return density.font(13);
  },
  get sm() {
    return density.font(15);
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
    return !platform.desktop;
  },
  height: 40,
  small: 36,
  compactHeight: 32,
  large: 46,
  icon: 36,
  get row() {
    return Math.max(52, density.line(20) * Math.max(1, fontScale()) + spacing.xl);
  },
  hitSlop(size: number) {
    return !platform.desktop ? Math.max(0, (44 - size) / 2) : 0;
  },
};
export const chrome = {
  header: 54,
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

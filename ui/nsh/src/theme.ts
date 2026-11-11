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

// Phone and Pad share the small size scale; desktop keeps NSH's standard scale.
// All colors come from nsh.json's theme so built-in Editor parts share them.
export const density = {
  get compact() {
    return !platform.desktop;
  },
  font(size: number) {
    return this.compact ? Math.max(11, Math.round(size * 0.9)) : size;
  },
  line(size: number) {
    return this.compact ? Math.max(14, size - 2) : size;
  },
  space(size: number) {
    return this.compact ? Math.round(size * 0.75) : size;
  },
  icon(size: number) {
    return this.compact ? Math.max(12, size - 2) : size;
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
  get sm() {
    return density.space(token("radius-small"));
  },
  get md() {
    return density.space(token("radius-control"));
  },
  get lg() {
    return density.space(token("radius"));
  },
  get xl() {
    return density.space(token("radius-card"));
  },
  get xxl() {
    return density.space(token("radius-dialog"));
  },
  full: 999,
};
export const typography = {
  get xs() {
    return density.font(12);
  },
  get sm() {
    return density.font(14);
  },
  get md() {
    return density.font(16);
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
    return density.compact;
  },
  get height() {
    return this.compact ? 40 : 32;
  },
  get small() {
    return this.compact ? 32 : 28;
  },
  get large() {
    return this.compact ? 44 : 40;
  },
  get icon() {
    return this.compact ? 36 : 32;
  },
  get row() {
    return Math.max(
      this.compact ? 44 : 40,
      density.line(20) * Math.max(1, fontScale()) + spacing.xl,
    );
  },
  hitSlop(size: number) {
    return this.compact ? Math.max(0, (44 - size) / 2) : 0;
  },
};
export const chrome = {
  get header() {
    return density.compact ? 48 : 64;
  },
  get bottom() {
    return density.compact ? 56 : 64;
  },
};

const names = {
  background: "background",
  panel: "card",
  text: "foreground",
  muted: "muted-foreground",
  border: "border",
  accent: "brand",
  primaryText: "on-brand",
  brandFill: "brand-fill",
  accentFill: "brand-tint",
  soft: "muted",
  hover: "hover",
  field: "field",
  fieldBorder: "input",
  raised: "raised",
  danger: "destructive",
  dangerText: "destructive-foreground",
  dangerFill: "destructive-tint",
  success: "up",
  warning: "warn",
  ring: "ring",
  sidebar: "sidebar",
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
export function surfaceFinish(kind: keyof typeof finishes.light) {
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
    borderRadius: radius.xl,
    borderCurve: "continuous",
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.popover,
    ...surfaceFinish("overlay"),
    transitionProperty: "opacity, transform",
    transitionDuration: reduceMotion() ? 0 : 120,
    transitionTimingFunction: "ease-out",
  };
}
export type StatusTone = "neutral" | "success" | "warning" | "danger";
export function statusColors(tone: StatusTone = "neutral") {
  const text =
    tone === "success"
      ? colors.success
      : tone === "warning"
        ? colors.warning
        : tone === "danger"
          ? colors.danger
          : colors.muted;
  const background = token(
    tone === "success"
      ? "up-tint"
      : tone === "warning"
        ? "warn-tint"
        : tone === "danger"
          ? "destructive-tint"
          : "muted",
  );
  return { text, background, border: colors.border };
}

import { token, type Tokens, type ViewStyle, colorScheme, platform, windowSize, fontScale, reduceMotion } from "nsh";
import finishes from "./finishes.json";

// NSH Gallery's theme and finishes, with compact Phone visuals and 44 pt hit areas.
// All colors come from nsh.json's theme so built-in Editor parts share them.
export const spacing = { xxs: 4, xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 24, section: 24, region: 32 };
export const radius = {
  get sm() { return token("radius-small"); },
  get md() { return token("radius-control"); },
  get lg() { return token("radius"); },
  get xl() { return token("radius-card"); },
  get xxl() { return token("radius-dialog"); },
  full: 999,
};
export const typography = { xs: 12, sm: 14, md: 16, lg: 18, xl: 20, xxl: 24 };
const touch = () => !platform.desktop || windowSize().width < 840;
export const control = {
  get compact() { return !platform.desktop; },
  get height() { return this.compact ? 40 : touch() ? 44 : 32; },
  get small() { return this.compact ? 32 : touch() ? 44 : 28; },
  get large() { return touch() ? 48 : 40; },
  get icon() { return this.compact ? 36 : touch() ? 44 : 32; },
  get row() { return Math.max(this.compact ? 48 : touch() ? 52 : 40, 20 * Math.max(1, fontScale()) + 24); },
  hitSlop(size: number) { return this.compact ? Math.max(0, (44 - size) / 2) : 0; },
};

const names = {
  background: "background", panel: "card", text: "foreground",
  muted: "muted-foreground", border: "border", accent: "brand",
  primaryText: "on-brand", brandFill: "brand-fill", accentFill: "brand-tint",
  soft: "muted", field: "field", fieldBorder: "input", raised: "raised",
  danger: "destructive", dangerText: "destructive-foreground", dangerFill: "destructive-tint",
  success: "up", warning: "warn", ring: "ring", sidebar: "sidebar",
  inset: "inset", line: "line", popover: "popover", scrim: "overlay-scrim",
  chartDownload: "chart-1", chartUpload: "chart-2",
} satisfies Record<string, keyof Tokens>;
const colors = {} as Record<keyof typeof names, string>;
for (const name of Object.keys(names) as (keyof typeof names)[])
  Object.defineProperty(colors, name, { enumerable: true, get: () => token(names[name]) });

export function palette() { return colors; }
export function surfaceFinish(kind: keyof typeof finishes.light) {
  return (colorScheme() === "dark" ? finishes.dark : finishes.light)[kind];
}
export function focusRing(): ViewStyle {
  return { outlineStyle: "solid", outlineWidth: 2, outlineColor: colors.ring, outlineOffset: -2 };
}
export function popupSurface(): ViewStyle {
  return { minWidth: 224, maxWidth: 480, padding: spacing.xxs,
    borderRadius: radius.xl, borderCurve: "continuous", borderWidth: 1,
    borderColor: colors.border, backgroundColor: colors.popover,
    ...surfaceFinish("overlay"), transitionProperty: "opacity, transform",
    transitionDuration: reduceMotion() ? 0 : 120, transitionTimingFunction: "ease-out" };
}
export type StatusTone = "neutral" | "success" | "warning" | "danger";
export function statusColors(tone: StatusTone = "neutral") {
  const text = tone === "success" ? colors.success : tone === "warning" ? colors.warning
    : tone === "danger" ? colors.danger : colors.muted;
  const background = token(tone === "success" ? "up-tint" : tone === "warning" ? "warn-tint"
    : tone === "danger" ? "destructive-tint" : "muted");
  return { text, background, border: colors.border };
}

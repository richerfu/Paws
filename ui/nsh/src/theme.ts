import { token, type Tokens, type ViewStyle, colorScheme } from "nsh";
import finishes from "./finishes.json";

// NSH Gallery's theme and surface finishes, with its 44 pt phone controls.
// All colors come from nsh.json's theme so built-in Editor parts share them.
export const spacing = { xxs: 4, xs: 6, sm: 8, md: 12, lg: 16, xl: 20, xxl: 24 };
export const radius = { sm: 6, md: 10, lg: 12, xl: 16, xxl: 20, full: 999 };
export const typography = { xs: 12, sm: 14, md: 16, lg: 18, xl: 20, xxl: 24 };
export const control = { height: 44, small: 36, large: 48, icon: 44 };

const names = {
  background: "background", panel: "card", text: "foreground",
  muted: "muted-foreground", border: "border", accent: "brand",
  primaryText: "on-brand", brandFill: "brand-fill", accentFill: "brand-tint",
  soft: "muted", field: "field", fieldBorder: "input", raised: "raised",
  danger: "destructive", dangerText: "destructive-foreground", dangerFill: "destructive-tint",
  success: "up", warning: "warn", ring: "info", sidebar: "sidebar",
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
export type StatusTone = "neutral" | "success" | "warning" | "danger";
export function statusColors(tone: StatusTone = "neutral") {
  const text = tone === "success" ? colors.success : tone === "warning" ? colors.warning
    : tone === "danger" ? colors.danger : colors.muted;
  const background = token(tone === "success" ? "up-tint" : tone === "warning" ? "warn-tint"
    : tone === "danger" ? "destructive-tint" : "muted");
  return { text, background, border: colors.border };
}

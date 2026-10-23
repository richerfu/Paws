import { colorScheme } from "nsh/device";

// Ported from arkit_shadcn/theme.rs: ThemePreset::Zinc, New York density.
export const spacing = { xxs: 4, xs: 6, sm: 8, md: 12, lg: 16, xl: 20, xxl: 24 };
export const radius = { sm: 4, md: 6, lg: 8, xl: 12, xxl: 16, full: 999 };
export const typography = { xs: 12, sm: 14, md: 16, lg: 18, xl: 20, xxl: 24 };
export const control = { height: 36, small: 32, large: 40, icon: 36 };
const dark = {
  background: "#09090b",
  panel: "#18181b",
  text: "#fafafa",
  muted: "#a1a1aa",
  border: "#27272a",
  accent: "#fafafa",
  primaryText: "#18181b",
  accentFill: "#27272a",
  soft: "#27272a",
  danger: "#7f1d1d",
  dangerText: "#fafafa",
  dangerFill: "#271416",
  success: "#4ade80",
  warning: "#fbbf24",
  ring: "#d4d4d8",
  sidebar: "#18181b",
};
const light = {
  background: "#ffffff",
  panel: "#ffffff",
  text: "#09090b",
  muted: "#71717a",
  border: "#e4e4e7",
  accent: "#09090b",
  primaryText: "#fafafa",
  accentFill: "#f4f4f5",
  soft: "#f4f4f5",
  danger: "#ef4444",
  dangerText: "#fafafa",
  dangerFill: "#fef2f2",
  success: "#16a34a",
  warning: "#d97706",
  ring: "#71717a",
  sidebar: "#f4f4f5",
};
export function palette() {
  return colorScheme() === "dark" ? dark : light;
}

export type StatusTone = "neutral" | "success" | "warning" | "danger";
const statusLight = {
  neutral: { text: "#52525b", background: "#f4f4f5", border: "#e4e4e7" },
  success: { text: "#166534", background: "#f0fdf4", border: "#bbf7d0" },
  warning: { text: "#92400e", background: "#fffbeb", border: "#fde68a" },
  danger: { text: "#991b1b", background: "#fef2f2", border: "#fecaca" },
};
const statusDark = {
  neutral: { text: "#d4d4d8", background: "#27272a", border: "#3f3f46" },
  success: { text: "#86efac", background: "#10291c", border: "#225b38" },
  warning: { text: "#fcd34d", background: "#30240e", border: "#705018" },
  danger: { text: "#fca5a5", background: "#321819", border: "#7f1d1d" },
};
export function statusColors(tone: StatusTone = "neutral") {
  return (colorScheme() === "dark" ? statusDark : statusLight)[tone];
}

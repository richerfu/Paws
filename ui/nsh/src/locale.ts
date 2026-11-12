import type { Preferences } from "./model";

export function isChinese(preferences: Preferences): boolean {
  return (
    preferences.language === "zh_cn" ||
    (preferences.language === "system" && preferences.systemLocale.toLowerCase().startsWith("zh"))
  );
}

export function bytes(value = 0): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let scaled = value / 1024,
    index = 0;
  while (scaled >= 1024 && index < units.length - 1) {
    scaled /= 1024;
    index++;
  }
  return `${scaled.toFixed(1)} ${units[index]}`;
}

export function includes(query: string, ...values: (string | undefined)[]): boolean {
  const needle = query.trim().toLocaleLowerCase();
  return !needle || values.some((value) => value?.toLocaleLowerCase().includes(needle));
}

export function timestamp(value?: string): string {
  if (!value) return "—";
  const numeric = Number(value);
  const milliseconds =
    numeric > 1e17
      ? numeric / 1e6
      : numeric > 1e14
        ? numeric / 1e3
        : numeric > 1e11
          ? numeric
          : numeric * 1000;
  const date = new Date(Number.isFinite(numeric) ? milliseconds : value);
  if (Number.isNaN(date.getTime())) return value;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export interface YamlSummary {
  lines: number;
  characters: number;
}

export function summarizeYaml(text: string): YamlSummary {
  // QuickJS runs these replacements in C. Avoid one JS array entry per code point.
  const newlines = text.replace(/[^\n]/g, "").length;
  const unpairedLength = text.replace(/[\uD800-\uDBFF][\uDC00-\uDFFF]/g, "").length;
  return {
    lines: text.length ? newlines + (text.endsWith("\n") ? 0 : 1) : 0,
    characters: (text.length + unpairedLength) / 2,
  };
}

import { computed, signal } from "nsh";
import { summarizeYaml } from "../src/yaml-summary";

export function yamlSummaryTrace() {
  const cases: Array<[string, number, number]> = [
    ["", 0, 0],
    ["a", 1, 1],
    ["\n", 1, 1],
    ["a\n", 1, 2],
    ["a\n\n", 2, 3],
    ["a\r\nb", 2, 4],
    ["中😀e\u0301\n", 1, 5],
    ["\ud800x\udc00", 1, 3],
    ["\ud800\udc00", 1, 1],
  ];
  for (const [text, lines, characters] of cases) {
    const actual = summarizeYaml(text);
    if (actual.lines !== lines || actual.characters !== characters)
      throw new Error("YAML line or Unicode code point count changed");
  }
  const text = signal("name: 节点😀\n"),
    dirty = signal(false),
    language = signal("zh");
  let scans = 0;
  const statistics = computed(() => {
    scans++;
    return summarizeYaml(text());
  });
  const label = computed(
    () => `${statistics().lines}/${statistics().characters}/${dirty()}/${language()}`,
  );
  label();
  dirty.set(true);
  label();
  language.set("en");
  label();
  if (scans !== 1) throw new Error("YAML statistics rescanned unchanged text for label changes");
  text.set("😀\n");
  label();
  if (Number(scans) !== 2 || statistics().characters !== 2)
    throw new Error("YAML edit did not refresh statistics");
  const large = "中😀\n".repeat(100000);
  const started = performance.now(),
    summary = summarizeYaml(large);
  if (summary.lines !== 100000 || summary.characters !== 300000)
    throw new Error("Full large document statistics were truncated");
  console.log(
    `PASS: full YAML statistics and text-only memo; syntheticUtf16=${large.length} elapsedMs=${performance.now() - started}`,
  );
}

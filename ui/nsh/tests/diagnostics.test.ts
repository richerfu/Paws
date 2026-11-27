import { expect, open, test } from "nsh/test";

async function fixture(width = 1280) {
  const probes: { group?: string; url: string }[] = [];
  const app = await open("./Diagnostics.tsx", {
    width, height: 900, platform: width === 1280 ? "windows" : "android", locale: "zh-CN",
    host: { handlers: { "paws.test.probe": (payload: { group?: string; url: string }) => { probes.push(payload); } } },
  });
  if (width === 1280) await app.press("id:desktop-nav-diagnostics");
  else {
    await app.press("id:nav-tools");
    await app.press("id:tool-diagnostics");
  }
  await app.waitUntil(() => app.enabled("id:diagnostics-probe"));
  return { app, probes };
}

for (const width of [390, 1280]) test(`diagnostics defaults to current routing with saved GLOBAL recovery (${width})`, async () => {
  const { app, probes } = await fixture(width);
  expect(app.shows("label:策略组")).toBe(false);
  expect(app.enabled("label:保存当前探测目标")).toBe(false);
  await app.press("id:diagnostics-probe");
  await app.waitUntil(() => probes.length === 1);
  expect(probes[0]).toEqual({ url: "https://www.gstatic.com/generate_204" });
  await app.waitFor("text:/^125 ms · 当前连接/");
  await app.press("label:检测路径");
  await app.press("id:select-option-group");
  await app.waitFor("label:策略组");
  expect(app.shows("text:GLOBAL → 套餐信息占位项")).toBe(true);
  await app.press("id:diagnostics-probe");
  await app.waitUntil(() => probes.length === 2);
  expect(probes[1].group).toBe("GLOBAL");
  await app.waitFor("id:diagnostics-probe-error");
  expect(app.shows("text:/^125 ms/")).toBe(false);
});

test("diagnostics discards a result received after reconnection", async () => {
  const { app, probes } = await fixture();
  await app.host.send("paws.test.hold", null);
  await app.press("id:diagnostics-probe");
  await app.waitUntil(() => probes.length === 1);
  await app.host.send("paws.test.reconnect", null);
  await app.wait(100);
  await app.host.send("paws.test.release", null);
  await app.waitUntil(() => app.enabled("id:diagnostics-probe"));
  expect(app.shows("text:最近探测")).toBe(false);
});

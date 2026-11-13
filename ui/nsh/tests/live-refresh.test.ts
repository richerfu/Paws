import { expect, open, test } from "nsh/test";

test("home keeps the last traffic sample while polling and resets on disconnect", async () => {
  let held = false;
  const app = await open("./LiveRefresh.tsx", { width: 390, height: 844, platform: "ios", locale: "zh-CN",
    host: { handlers: { "paws.test.live.held": () => { held = true; } } } });
  await app.waitFor("id:home-connection-action");
  expect(app.shows("id:vpn-floating-area")).toBe(false);
  expect(app.shows("id:vpn-toggle")).toBe(false);
  expect(app.texts()).toContain("32.0");
  await app.host.send("paws.test.live.hold", null);
  await app.waitUntil(() => held);
  expect(app.texts()).toContain("32.0");
  await app.host.send("paws.test.live.release", null);
  await app.waitUntil(() => app.texts().includes("64.0"));
  held = false;
  await app.host.send("paws.test.live.hold", null);
  await app.waitUntil(() => held);
  await app.host.send("paws.test.live.fail", null);
  await app.waitFor("id:home-traffic-retry");
  expect(app.texts()).toContain("64.0");
  await app.press("id:home-traffic-retry");
  await app.waitUntil(() => !app.shows("id:home-traffic-retry"));
  await app.host.send("paws.test.live.disconnect", null);
  await app.waitUntil(() => !app.texts().includes("64.0"));
});

test("connection sheet keeps its native owner while live details update", async () => {
  const app = await open("./LiveRefresh.tsx", { width: 390, height: 844, platform: "ios", locale: "zh-CN" });
  await app.press("id:nav-tools");
  await app.press("id:tool-connections");
  await app.press("id:activity-row-connection-connection-1");
  await app.wait(250);
  console.log("LIVE SHEET BEGIN");
  for (let index = 0; index < 3; index++) {
    await app.host.send("paws.test.live.tick", null);
    await app.wait(50);
    expect(app.shows("id:activity-close-connection-connection-1")).toBe(true);
  }
  console.log("LIVE SHEET END");
  expect(app.texts()).toContain("↓ 5.0 KB · ↑ 1.0 KB");
  await app.press("id:rule-for-connection-1");
  await app.waitFor("id:manual-rule-value");
});

test("provider dialog keeps its native content while member delays update", async () => {
  const app = await open("./LiveRefresh.tsx", { width: 390, height: 844, platform: "ios", locale: "zh-CN" });
  await app.press("id:nav-tools");
  await app.press("id:tool-resources");
  await app.press("id:provider-menu-proxy-Test provider");
  await app.press("id:provider-details-Test provider");
  await app.wait(250);
  console.log("LIVE PROVIDER BEGIN");
  for (let index = 0; index < 3; index++) {
    await app.host.send("paws.test.live.tick", null);
    await app.wait(50);
    expect(app.shows("id:dialog-close")).toBe(true);
  }
  console.log("LIVE PROVIDER END");
});

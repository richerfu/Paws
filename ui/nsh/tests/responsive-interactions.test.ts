import { expect, open, test } from "nsh/test";

test("desktop commands, context menus and drafts survive window resizing", async () => {
  const reports: string[] = [];
  const app = await open("./Preview.tsx", {
    width: 1280, height: 900, platform: "windows", locale: "zh-CN",
    host: { handlers: { "paws.test.report": (value: { command: string }) => reports.push(value.command) } },
  });
  expect(app.shows("id:desktop-sidebar")).toBe(true);
  expect(app.shows("id:bottom-navigation")).toBe(false);
  await app.keyboard.press("mod+2");
  await app.waitFor("id:profile-menu-test-profile");
  await app.rightClick("id:activate-profile-test-profile");
  await app.waitFor("id:edit-subscription-test-profile");
  await app.press("id:edit-subscription-test-profile");
  await app.waitFor("id:save-subscription");
  await app.setViewport(620, 760);
  expect(app.shows("id:save-subscription")).toBe(true);
  await app.keyboard.press("escape");
  await app.waitFor("id:add-profile");
  await app.setViewport(1280, 900);
  await app.press("id:desktop-nav-settings");
  await app.waitFor("id:save-vpn");
  expect(app.enabled("id:save-vpn")).toBe(false);
  await app.press("label:DNS 劫持");
  expect(app.enabled("id:save-vpn")).toBe(true);
  expect(reports.filter((command) => command === "settings.vpn")).toHaveLength(0);
  await app.setViewport(390, 844);
  expect(app.shows("id:desktop-sidebar")).toBe(false);
  expect(app.enabled("id:save-vpn")).toBe(true);
  await app.setViewport(1280, 900);
  await app.keyboard.press("mod+1");
  expect(app.dialogs().at(-1)!.buttons).toEqual(["取消", "丢弃草稿"]);
  expect(app.enabled("id:save-vpn")).toBe(true);
  await app.press("id:save-vpn");
  expect(reports.filter((command) => command === "settings.vpn")).toHaveLength(1);
});

test("reduced motion preserves disclosure, native menu selection and Escape", async () => {
  const app = await open("./Preview.tsx", { width: 390, height: 844, reducedMotion: true, locale: "zh-CN" });
  await app.press("id:summary-group-Proxy");
  await app.waitFor("id:summary-select-proxy-Test node");
  await app.press("id:summary-group-Proxy");
  expect(app.shows("id:summary-select-proxy-Test node")).toBe(false);
  await app.press("id:nav-tools");
  await app.press("id:tool-settings");
  await app.press("id:vpn-engine");
  await app.press("id:select-option-mihomo");
  await app.waitFor("id:save-vpn");
  expect(app.shows("text:mihomo")).toBe(true);
  await app.press("id:vpn-engine");
  await app.keyboard.press("escape");
  expect(app.shows("id:select-popover")).toBe(false);
  expect(app.enabled("id:save-vpn")).toBe(true);
});

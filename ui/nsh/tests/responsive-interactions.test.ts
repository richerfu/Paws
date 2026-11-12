import { expect, open, test } from "nsh/test";

for (const device of [
  { name: "Phone", platform: "ios" as const, width: 390 },
  { name: "Pad", platform: "ios" as const, width: 1024 },
  { name: "PC", platform: "windows" as const, width: 1280 },
])
  test(`${device.name} preserves form drafts and menu input while resizing`, async () => {
    const app = await open("./Preview.tsx", {
      platform: device.platform,
      width: device.width,
      height: 900,
      locale: "zh-CN",
    });
    if (device.name === "Phone") {
      await app.press("id:nav-tools");
      await app.press("id:tool-settings");
    } else await app.press("id:desktop-nav-settings");
    await app.type("id:vpn-mtu", "1");
    expect(app.value("id:vpn-mtu")).toBe("15001");
    expect(app.enabled("id:save-vpn")).toBe(true);
    await app.setViewport(620, 900);
    expect(app.value("id:vpn-mtu")).toBe("15001");
    expect(app.enabled("id:save-vpn")).toBe(true);
    await app.press("id:vpn-engine");
    expect(app.shows("id:select-option-meow")).toBe(true);
    await app.keyboard.press("escape");
  });

test("desktop commands, context menus and drafts survive window resizing", async () => {
  const reports: string[] = [];
  const app = await open("./Preview.tsx", {
    width: 1280,
    height: 900,
    platform: "windows",
    locale: "zh-CN",
    host: {
      handlers: {
        "paws.test.report": (value: { command: string }) =>
          reports.push(value.command),
      },
    },
  });
  expect(app.shows("id:desktop-sidebar")).toBe(true);
  expect(app.shows("id:desktop-dashboard-workspace")).toBe(true);
  expect(app.shows("id:bottom-navigation")).toBe(false);
  expect(app.shows("id:nav-tools")).toBe(false);
  expect(app.shows("id:desktop-nav-requests")).toBe(false);
  await app.keyboard.press("mod+2");
  await app.waitFor("id:profile-menu-test-profile");
  await app.rightClick("id:open-profile-test-profile");
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
  expect(reports.filter((command) => command === "settings.vpn")).toHaveLength(
    0,
  );
  await app.setViewport(390, 844);
  expect(app.shows("id:desktop-sidebar")).toBe(true);
  expect(app.shows("id:bottom-navigation")).toBe(false);
  expect(app.enabled("id:save-vpn")).toBe(true);
  await app.setViewport(1280, 900);
  await app.keyboard.press("mod+1");
  expect(app.dialogs().at(-1)!.buttons).toEqual(["继续编辑", "丢弃草稿"]);
  expect(app.enabled("id:save-vpn")).toBe(true);
  await app.press("id:save-vpn");
  expect(reports.filter((command) => command === "settings.vpn")).toHaveLength(
    1,
  );
  await app.keyboard.press("mod+3");
  await app.waitFor("id:desktop-proxy-workspace");
  await app.press("id:desktop-group-Proxy");
  await app.waitFor("id:select-proxy-Test node");
  await app.press("id:desktop-nav-connections");
  await app.waitFor("id:activity-table-header");
  await app.setViewport(620, 760);
  expect(app.shows("id:activity-table-header")).toBe(true);
  expect(app.shows("id:desktop-sidebar")).toBe(true);
  expect(app.shows("id:nav-back")).toBe(false);
  await app.press("label:请求历史");
  expect(app.shows("id:activity-table-header")).toBe(true);
  await app.press("id:desktop-toggle-sidebar");
  expect(app.shows("id:desktop-sidebar")).toBe(true);
});

test("reduced motion preserves disclosure, native menu selection and Escape", async () => {
  const app = await open("./Preview.tsx", {
    width: 390,
    height: 844,
    reducedMotion: true,
    locale: "zh-CN",
  });
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

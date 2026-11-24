import { expect, open, test } from "nsh/test";
import { workspaceLayout } from "../src/layout";

// The 840 pt breakpoint alone is insufficient after sidebar/padding/font growth.
test("pane policy respects sidebar, page cap and enlarged text minima", () => {
  expect(workspaceLayout(840, false, false).masterDetail).toBe(true);
  expect(workspaceLayout(840, false, true).usable).toBe(568);
  expect(workspaceLayout(840, false, true).masterDetail).toBe(true);
  expect(workspaceLayout(840, false, true, 1.3).masterDetail).toBe(false);
  expect(workspaceLayout(1024, false, true, 1.3).masterDetail).toBe(true);
  expect(workspaceLayout(1024, false, true, 1.6).masterDetail).toBe(false);
  expect(workspaceLayout(1440, true, true, 1.6, 960).overview).toBe(false);
  for (const scale of [1, 1.3, 1.6]) {
    for (const width of [320, 360, 390, 430, 600, 640, 800, 840, 1024, 1100, 1280, 1440]) {
      for (const desktop of [false, true]) {
        const layout = workspaceLayout(width, desktop, true, scale, 1120);
        if (layout.masterDetail) {
          expect(layout.collection >= layout.collectionMin).toBe(true);
          expect(layout.usable - layout.collection - layout.gap >= layout.detailMin).toBe(true);
        }
      }
    }
  }
});

test("Phone detail navigation returns to its actual source and menus preserve commands", async () => {
  const commands: string[] = [];
  const app = await open("./Preview.tsx", { platform: "ios", width: 390, height: 844, locale: "zh-CN", host: { handlers: { "paws.test.report": (value: {command: string}) => commands.push(value.command) } } });
  await app.press("id:nav-profiles");
  await app.press("id:profiles-more");
  expect(app.enabled("id:refresh-profiles")).toBe(true);
  await app.press("id:refresh-profiles");
  await app.waitUntil(() => commands.includes("profiles.refreshAll"));
  await app.waitFor("id:dismiss-toast-1");
  await app.press("id:dismiss-toast-1");
  await app.waitFor("id:sonner-viewport", { state: "gone" });
  await app.press("id:open-profile-test-profile");
  await app.press("id:profile-proxies");
  await app.press("id:group-Proxy");
  await app.type("id:proxies-search", "Test");
  await app.setViewport(1024, 768);
  expect(app.value("id:proxies-search")).toBe("Test");
  expect(app.shows("id:select-proxy-Test node")).toBe(true);
  await app.setViewport(390, 844);
  await app.press("id:nav-back");
  await app.press("id:nav-back");
  expect(app.shows("id:profile-proxies")).toBe(true);
  await app.press("id:nav-back");
  await app.press("id:nav-dashboard");
  await app.press("id:all-proxies");
  await app.press("id:nav-back");
  expect(app.shows("id:home-connection-action")).toBe(true);
});

test("workspace switching restores the detail without manufacturing Back history", async () => {
  const app = await open("./Preview.tsx", { platform: "windows", width: 1280, height: 900, locale: "zh-CN" });
  await app.press("id:nav-profiles");
  await app.press("id:open-profile-test-profile");
  await app.type("id:profiles-search", "Test");
  await app.press("id:desktop-nav-connections");
  expect(app.shows("id:nav-back")).toBe(false);
  await app.press("id:nav-profiles");
  expect(app.value("id:profiles-search")).toBe("Test");
  expect(app.shows("id:profile-proxies")).toBe(true);
  await app.press("id:nav-back");
  expect(app.shows("id:nav-back")).toBe(false);
});

test("Escape respects an import draft and workspace shortcuts yield to the modal", async () => {
  const app = await open("./Preview.tsx", { platform: "windows", width: 1280, height: 900, locale: "zh-CN" });
  await app.press("id:nav-profiles");
  await app.press("id:add-profile");
  await app.type("id:profile-input", "https://example.com/new");
  await app.keyboard.press("mod+1");
  expect(app.shows("id:profile-input")).toBe(true);
  await app.keyboard.press("escape");
  expect(app.dialogs().at(-1)!.buttons).toEqual(["继续编辑", "放弃修改"]);
  expect(app.value("id:profile-input")).toBe("https://example.com/new");
  app.onDialog(dialog => dialog.buttons[dialog.buttons.length - 1]);
  await app.keyboard.press("escape");
  await app.wait(250);
  expect(app.shows("id:profile-input")).toBe(false);
  await app.press("id:add-profile");
  expect(app.value("id:profile-input")).toBe("");
});

test("native Back closes overlays, pops subpages, and yields to system at root workspaces", async () => {
  const app = await open("./Preview.tsx", {
    platform: "android",
    width: 390,
    height: 844,
    locale: "zh-CN",
  });
  await app.waitFor("id:home-connection-action");
  await expect(app.back()).rejects.toMatchObject({ code: "ERR_NSH_TEST_NO_BACK" });

  await app.press("id:all-proxies");
  await app.waitFor("id:group-Proxy");
  await app.back();
  await app.waitFor("id:home-connection-action");
  await expect(app.back()).rejects.toMatchObject({ code: "ERR_NSH_TEST_NO_BACK" });

  await app.press("id:nav-profiles");
  await app.waitFor("id:add-profile");
  await app.press("id:add-profile");
  await app.waitFor("id:profile-input");
  await app.back();
  await app.wait(250);
  expect(app.shows("id:profile-input")).toBe(false);
  await expect(app.back()).rejects.toMatchObject({ code: "ERR_NSH_TEST_NO_BACK" });

  await app.setViewport(1024, 768);
  await app.press("id:desktop-nav-settings");
  await app.waitFor("id:save-vpn");
  await expect(app.back()).rejects.toMatchObject({ code: "ERR_NSH_TEST_NO_BACK" });
  await app.setViewport(390, 844);
  await app.back();
  await app.waitFor("id:tool-settings");
  await expect(app.back()).rejects.toMatchObject({ code: "ERR_NSH_TEST_NO_BACK" });
});

test("dashboard telemetry, proxy group title, controller engine and profile shortcuts work end to end", async () => {
  const app = await open("./Preview.tsx", {
    platform: "android",
    width: 390,
    height: 844,
    locale: "zh-CN",
  });
  await app.waitFor("id:home-connection-action");
  expect(app.shows("id:home-engine-badge")).toBe(false);
  expect(app.shows("id:home-node-delay")).toBe(true);
  expect(app.shows("text:42 ms")).toBe(true);

  await app.press("id:home-connection-action");
  await app.waitFor("id:home-engine-badge");
  expect(app.shows("text:Meow")).toBe(true);
  expect(app.shows("text:/Test profile · 00:/")).toBe(true);

  await app.press("id:all-proxies");
  await app.waitFor("id:group-Proxy");
  await app.press("id:group-Proxy");
  await app.waitFor("id:select-proxy-Test node");
  expect(app.shows("text:Proxy")).toBe(true);
  expect(app.shows("text:分组节点")).toBe(false);
  await app.press("id:nav-back");
  await app.press("id:nav-back");

  await app.press("id:nav-profiles");
  await app.waitFor("id:profiles-quick-import");
  expect(app.shows("id:profiles-quick-yaml")).toBe(true);
  await app.press("id:profiles-quick-import");
  await app.waitFor("id:profile-input");
  await app.press("id:dialog-close");
  await app.wait(200);
  await app.press("id:profiles-quick-yaml");
  await app.waitFor("id:validate-profile");
  await app.press("id:nav-back");

  await app.setViewport(1280, 900);
  await app.waitFor("id:open-profile-test-profile");
  await app.press("id:open-profile-test-profile");
  await app.waitFor("id:profile-edit-yaml-detail");
  expect(app.shows("id:profiles-quick-actions")).toBe(false);
  expect(app.shows("id:profile-export-detail")).toBe(true);
  expect(app.shows("id:profile-engine-settings")).toBe(true);
  await app.press("id:profile-engine-settings");
  await app.waitFor("id:save-vpn");

  await app.press("id:nav-traffic");
  await app.waitFor("text:当前内核");
  expect(app.shows("text:Meow")).toBe(true);
});

test("switching to global mode on Overview remounts GLOBAL cleanly at top and switches its node", async () => {
  const app = await open("./GlobalMode.tsx", {
    platform: "android",
    width: 390,
    height: 844,
    locale: "zh-CN",
  });
  await app.waitFor("id:mode-global");
  await app.press("id:mode-global");
  await app.waitFor("text:全局模式");
  expect(app.shows("id:summary-group-GLOBAL")).toBe(true);
  expect(app.shows("text:全局节点")).toBe(true);
  expect(app.shows("text:手动选择 · DIRECT")).toBe(true);

  await app.press("id:summary-group-GLOBAL");
  await app.waitFor("id:summary-select-proxy-Node B");
  await app.press("id:summary-select-proxy-Node B");
  await app.waitFor("text:手动选择 · Node B");
  expect(app.shows("text:Node B")).toBe(true);

  await app.press("id:all-proxies");
  await app.waitFor("id:group-GLOBAL");
  expect(app.shows("text:全局节点")).toBe(true);
  await app.press("id:group-GLOBAL");
  await app.waitFor("id:select-proxy-Node A");
  expect(app.shows("text:全局节点")).toBe(true);
});

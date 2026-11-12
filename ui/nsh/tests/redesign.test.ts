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

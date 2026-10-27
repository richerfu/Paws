import { expect, open, test, type App } from "nsh/test";

async function fixture(width = 390, scheme: "light" | "dark" = "light") {
  const reports: { command: string; content?: string; expectedRevision?: number; profileId?: string }[] = [];
  const replies = new Map<number, { error?: string }>();
  const app = await open("./ProfileEditorPreview.tsx", {
    width, height: 844, colorScheme: scheme, locale: "zh-CN",
    host: { handlers: {
      "paws.test.register": null,
      "paws.test.done": (reply: { token: number; error?: string }) => { replies.set(reply.token, reply); },
      "paws.test.report": (report: typeof reports[number]) => { reports.push(report); },
    } },
  });
  let token = 0;
  const action = async (name: string) => {
    const current = ++token;
    await app.host.send("paws.test.action", { id: `editor-fixture-${name}`, token: current });
    await app.waitUntil(() => replies.has(current));
    if (replies.get(current)?.error) throw new Error(replies.get(current)!.error);
  };
  return { app, reports, action };
}
async function openEditor(app: App) {
  await app.press("id:nav-profiles");
  await app.press("id:profile-menu-test-profile");
  await app.press("id:edit-yaml-test-profile");
  await app.waitFor("id:profile-editor-page");
  expect(app.shows("id:dialog-close")).toBe(false);
  expect(app.shows("id:bottom-navigation")).toBe(false);
}
async function append(app: App) {
  await app.press("text:rules: []");
  await app.keyboard.press("mod+down");
  await app.keyboard.press("x");
  await app.waitFor("text:未保存");
}

test("YAML opens on a full page, finds text, validates and saves exact normalized edits", async () => {
  const { app, reports } = await fixture();
  await openEditor(app);
  await app.waitFor("text:proxies: []");
  await app.press("id:editor-find");
  await app.type("id:editor-search", "rules");
  await app.waitFor("text:1 处匹配");
  await app.press("id:editor-find-next");
  await app.press("id:editor-find");
  await append(app);
  await app.press("id:validate-profile");
  await app.press("id:save-profile");
  await app.waitFor("id:add-profile");
  const saved = reports.filter((r) => r.command === "profiles.save");
  expect(saved).toHaveLength(1);
  expect(saved[0].profileId).toBe("test-profile");
  expect(saved[0].expectedRevision).toBe(1);
  expect(saved[0].content).toBe("proxies: []\nrules: []\nx");
});

test("unsaved edits guard native Back and desktop navigation; cancel and undo retain the draft", async () => {
  const { app } = await fixture(1280, "dark");
  await openEditor(app);
  await append(app);
  await app.back();
  await app.waitFor("id:cancel-confirmation");
  await app.press("id:cancel-confirmation");
  expect(app.shows("id:profile-yaml-editor")).toBe(true);
  await app.press("id:nav-tools");
  await app.waitFor("id:cancel-confirmation");
  await app.press("id:cancel-confirmation");
  await app.press("text:x");
  await app.keyboard.press("mod+z");
  await app.waitFor("text:YAML");
  await app.back();
  await app.waitFor("id:add-profile");
});

test("failed reads retry, and failed validation/save keep the edited page available", async () => {
  const { app, reports, action } = await fixture();
  await action("fail-read");
  await openEditor(app);
  await app.waitFor("id:retry-profile-read");
  expect(app.enabled("id:save-profile")).toBe(false);
  await app.press("id:retry-profile-read");
  await app.waitFor("text:rules: []");
  await append(app);
  await action("fail-validation");
  await app.press("id:validate-profile");
  await app.waitFor("text:Synthetic YAML validation failure");
  await action("fail-save");
  await app.press("id:save-profile");
  await app.waitFor("text:Synthetic save failure");
  expect(app.shows("text:未保存")).toBe(true);
  await app.press("id:save-profile");
  await app.waitFor("id:add-profile");
  expect(reports.filter((r) => r.command === "profiles.save")).toHaveLength(2);
});

test("a remote config revision blocks saving until an explicit discard and reload", async () => {
  const { app, reports, action } = await fixture();
  await openEditor(app);
  await append(app);
  await action("remote-change");
  await app.waitFor("id:reload-profile");
  expect(app.enabled("id:save-profile")).toBe(false);
  expect(reports.filter((r) => r.command === "profiles.save")).toHaveLength(0);
  await app.press("id:reload-profile");
  await app.press("id:cancel-confirmation");
  expect(app.shows("text:未保存")).toBe(true);
  await app.press("id:reload-profile");
  await app.press("id:confirm-action");
  await app.waitFor("text:# refreshed remotely");
  expect(app.enabled("id:save-profile")).toBe(true);
  await app.press("id:save-profile");
  await app.waitFor("id:add-profile");
  expect(reports.filter((r) => r.command === "profiles.save")[0].expectedRevision).toBe(2);
});

test("a held save blocks leaving and a large YAML document keeps its exact source", async () => {
  const { app, reports, action } = await fixture(320);
  await action("large-document");
  await openEditor(app);
  await app.waitFor("text:proxies: []");
  await app.press("id:editor-wrap");
  await action("hold-save");
  await app.press("id:save-profile");
  await app.back();
  expect(app.shows("id:profile-editor-page")).toBe(true);
  expect(app.enabled("id:validate-profile")).toBe(false);
  await action("release-save");
  await app.waitFor("id:add-profile");
  expect(reports.filter((r) => r.command === "profiles.save")[0].content)
    .toBe("proxies: []\nrules: []\n" + "# synthetic YAML 0123456789\n".repeat(4000));
});

test("Back leaves a pending read and a late result cannot reopen the disposed editor", async () => {
  const { app, reports, action } = await fixture();
  await action("hold-read");
  await openEditor(app);
  expect(app.enabled("id:save-profile")).toBe(false);
  await app.back();
  await app.waitFor("id:add-profile");
  await action("release-read");
  expect(app.shows("id:profile-editor-page")).toBe(false);
  await app.press("id:profile-menu-test-profile");
  await app.press("id:edit-yaml-test-profile");
  await app.waitFor("text:rules: []");
  expect(reports.filter((r) => r.command === "profiles.read")).toHaveLength(2);
});

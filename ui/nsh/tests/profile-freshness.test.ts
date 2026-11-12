import { expect, open, test } from "nsh/test";

async function fixture() {
  let readiness = { fresh: false, ready: false, config: false, configRevision: 0 };
  let blocked = 0;
  const commands: string[] = [];
  const app = await open("./ProfileFreshness.tsx", {
    width: 1280,
    platform: "windows",
    height: 844,
    locale: "zh-CN",
    host: {
      handlers: {
        "paws.test.readiness": (value: typeof readiness) => {
          readiness = value;
        },
        "paws.test.blocked": () => {
          blocked++;
        },
        "paws.test.command": (command: string) => {
          commands.push(command);
        },
      },
    },
  });
  await app.press("id:nav-profiles");
  await app.waitUntil(() => readiness.ready);
  return { app, commands, readiness: () => readiness, blocked: () => blocked };
}

test("profile polling keeps menu and edit actions enabled while counters refresh", async () => {
  const { app, commands, readiness } = await fixture();
  await app.press("id:profile-menu-test-profile");
  await app.wait(250);
  for (let index = 0; index < 3; index++) {
    await app.host.send("paws.test.invalidate", false);
    await app.waitUntil(() => !readiness().fresh);
    expect(readiness().ready).toBe(true);
    expect(app.enabled("id:edit-subscription-test-profile")).toBe(true);
    expect(app.enabled("id:refresh-profiles")).toBe(true);
    await app.host.send("paws.test.release", "profiles");
    await app.waitUntil(() => readiness().fresh);
  }
  await app.press("id:edit-subscription-test-profile");
  await app.waitFor("id:save-subscription");
  await app.host.send("paws.test.invalidate", false);
  await app.waitUntil(() => !readiness().fresh);
  expect(app.enabled("id:save-subscription")).toBe(true);
  await app.host.send("paws.test.attempt", null);
  await app.waitUntil(() => commands.length === 1);
  expect(commands).toEqual(["profiles.subscription"]);
  await app.host.send("paws.test.release", "profiles");
});

test("config changes and failed profile reads still block stale UI and direct commands", async () => {
  const { app, commands, readiness, blocked } = await fixture();
  await app.press("id:profile-menu-test-profile");
  await app.wait(250);
  await app.press("id:edit-subscription-test-profile");
  await app.waitFor("id:save-subscription");
  await app.host.send("paws.test.invalidate", true);
  await app.waitUntil(() => !readiness().config);
  expect(app.enabled("id:save-subscription")).toBe(false);
  await app.host.send("paws.test.attempt", null);
  await app.waitUntil(() => blocked() === 1);
  expect(commands).toEqual([]);
  await app.host.send("paws.test.release", "config");
  await app.waitUntil(() => readiness().config && readiness().configRevision === 2);
  expect(readiness().ready).toBe(false);
  await app.host.send("paws.test.attempt", null);
  await app.waitUntil(() => blocked() === 2);
  await app.host.send("paws.test.release", "profiles");
  await app.waitUntil(() => readiness().ready);
  // The open editor still has its original revision and must not save it.
  expect(app.enabled("id:save-subscription")).toBe(false);
  await app.press("id:dialog-close");
  await app.wait(250);
  await app.press("id:profile-menu-test-profile");
  await app.wait(250);
  await app.host.send("paws.test.invalidate", false);
  await app.waitUntil(() => !readiness().fresh);
  await app.host.send("paws.test.fail-profiles", null);
  await app.waitUntil(() => !readiness().ready);
  expect(app.shows("id:edit-subscription-test-profile")).toBe(false);
  expect(app.enabled("id:refresh-profiles")).toBe(false);
  await app.host.send("paws.test.attempt", null);
  await app.waitUntil(() => blocked() === 3);
  await app.host.send("paws.test.retry", null);
  await app.waitUntil(() => readiness().ready);
  await app.wait(250);
  await app.press("id:profile-menu-test-profile");
  await app.waitFor("id:edit-subscription-test-profile");
  expect(app.enabled("id:edit-subscription-test-profile")).toBe(true);
  expect(commands).toEqual([]);
});

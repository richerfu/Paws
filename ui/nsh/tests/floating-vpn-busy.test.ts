import { expect, open, test } from "nsh/test";

test("a busy VPN action leaves its floating control draggable and blocks taps", async () => {
  let busy = false;
  const commands: string[] = [];
  const app = await open("./FloatingHitPreview.tsx", {
    width: 390, height: 844, locale: "zh-CN",
    safeArea: { top: 47, bottom: 34, left: 0, right: 0 },
    host: { handlers: {
      "paws.test.busy": (value: boolean) => { busy = value; },
      "paws.test.report": (value: { command: string }) => { commands.push(value.command); },
    } },
  });
  await app.waitFor("id:vpn-toggle");
  await app.host.send("paws.test.hold-action", true);
  await app.waitUntil(() => busy);
  await app.drag("id:vpn-toggle", { x: -220, y: -180 });
  await app.wait(600);
  await expect(app.press("id:vpn-toggle")).rejects.toMatchObject({ code: "ERR_NSH_TEST_DISABLED" });
  expect(commands).toEqual([]);
  await app.host.send("paws.test.hold-action", false);
  await app.waitUntil(() => !busy);
  await app.drag("id:vpn-toggle", { x: 240, y: 100 });
  await app.wait(600);
  await app.press("id:vpn-toggle");
  expect(commands).toEqual(["vpn.start"]);
});

import { expect, open, test } from "nsh/test";

async function fixture(width = 390, height = 844) {
  const commands: string[] = [];
  const app = await open("./Preview.tsx", {
    width,
    height,
    locale: "zh-CN",
    safeArea: { top: 47, bottom: 34, left: 0, right: 0 },
    host: {
      handlers: {
        "paws.test.report": (report: { command: string }) => {
          commands.push(report.command);
        },
      },
    },
  });
  await app.waitFor("id:vpn-toggle");
  return { app, commands };
}

test("native drags never toggle VPN; taps after docking still start and stop", async () => {
  const { app, commands } = await fixture();
  await app.drag("id:vpn-toggle", { x: -80, y: -140 });
  await app.wait(600);
  await app.drag("id:vpn-toggle", { x: -240, y: -150 });
  await app.wait(600);
  expect(commands).toEqual([]);
  await app.press("id:vpn-toggle");
  await app.waitFor("label:停止 VPN");
  await app.drag("id:vpn-toggle", { x: 260, y: 180 });
  await app.wait(600);
  expect(commands).toEqual(["vpn.start"]);
  await app.press("id:vpn-toggle");
  await app.waitFor("label:启动 VPN");
  expect(commands).toEqual(["vpn.start", "vpn.stop"]);
});

test("out-of-bounds drags and viewport changes keep the control available", async () => {
  const { app, commands } = await fixture(320, 568);
  await app.drag("id:vpn-toggle", { x: -600, y: -800 });
  await app.wait(600);
  await app.setViewport(740, 390);
  await app.drag("id:vpn-toggle", { x: 900, y: 600 });
  await app.wait(600);
  await app.setViewport(320, 568);
  expect(app.count("id:vpn-toggle")).toBe(1);
  expect(commands).toEqual([]);
  await app.press("id:vpn-toggle");
  expect(commands).toEqual(["vpn.start"]);
});

test("the floating area lets navigation through and yields to the inline desktop button", async () => {
  const { app, commands } = await fixture();
  await app.drag("id:vpn-toggle", { x: -250, y: -220 });
  await app.wait(600);
  await app.press("id:nav-profiles");
  expect(app.shows("id:vpn-floating-area")).toBe(false);
  await app.press("id:nav-dashboard");
  expect(app.shows("id:vpn-floating-area")).toBe(true);
  await app.setViewport(1280, 900);
  expect(app.shows("id:vpn-floating-area")).toBe(false);
  expect(app.count("id:vpn-toggle")).toBe(0);
  expect(app.shows("id:home-connection-action")).toBe(true);
  await app.press("id:home-connection-action");
  await app.setViewport(390, 844);
  expect(app.shows("id:vpn-floating-area")).toBe(true);
  await app.press("id:vpn-toggle");
  expect(commands).toEqual(["vpn.start", "vpn.stop"]);
});

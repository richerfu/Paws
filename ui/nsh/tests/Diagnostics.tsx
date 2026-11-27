import { cleanup, host, mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest, UiState } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

mount(() => {
  const client = new CoreClient();
  let state: UiState = { ...fixture(), vpnRunning: true, vpnSessionId: "probe-session", activeEngine: "mihomo", vpnLifecycle: "connected" };
  state.proxyGroups.unshift({
    name: "GLOBAL", groupType: "Selector", selected: "套餐信息占位项", fixed: null,
    proxies: [{ name: "套餐信息占位项", proxyType: "Shadowsocks", selected: true }],
  });
  const server = new DomainHost(client, () => state);
  let hold = false;
  let pending: CoreRequest | undefined;
  server.onCommand = (request) => {
    if (request.command === "diagnostics.healthGet") server.reply(request, null);
    else if (request.command === "diagnostics.watchGet") server.reply(request, {
      enabled: true, group: "GLOBAL", url: "https://www.gstatic.com/generate_204",
    });
    else if (request.command === "diagnostics.probe") {
      host.post("paws.test.probe", request.payload);
      if (hold) pending = request;
      else if (request.payload.group) server.result({
        id: request.id, ok: false,
        error: "selected proxy could not complete the target URL test (HTTP 503)",
      });
      else server.reply(request, { delay: 125, sessionId: state.vpnSessionId });
    } else throw new Error(`Unexpected diagnostic command: ${request.command}`);
  };
  cleanup(host.on("paws.test.hold", () => { hold = true; }));
  cleanup(host.on("paws.test.reconnect", () => {
    state = { ...state, revision: state.revision + 1, vpnSessionId: "replacement-session" };
    server.change(["status"]);
  }));
  cleanup(host.on("paws.test.release", () => {
    if (pending) server.reply(pending, { delay: 999, sessionId: "probe-session" });
    pending = undefined;
  }));
  server.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

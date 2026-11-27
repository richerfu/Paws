import { mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { DomainHost } from "./DomainHost";
import type { DomainName } from "../src/model";
import { fixture } from "./fixture";

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  state.proxyGroups = [
    ...Array.from({ length: 8 }, (_, index) => ({
      name: `Group ${index + 1}`,
      groupType: "URLTest",
      fixed: "",
      selected: "Node A",
      proxies: [
        { name: "Node A", proxyType: "Shadowsocks", delayMs: 38, selected: true },
        { name: "Node B", proxyType: "Trojan", delayMs: 52, selected: false },
      ],
    })),
    {
      name: "GLOBAL",
      groupType: "Selector",
      fixed: "",
      selected: "DIRECT",
      proxies: [
        { name: "DIRECT", proxyType: "Direct", selected: true },
        { name: "Group 1", proxyType: "URLTest", selected: false },
        { name: "Node A", proxyType: "Shadowsocks", delayMs: 38, selected: false },
        { name: "Node B", proxyType: "Trojan", delayMs: 52, selected: false },
      ],
    },
  ];
  const host = new DomainHost(client, () => state);
  host.onCommand = (request) => {
    let changed: DomainName[] = [];
    if (request.command === "mode.set") {
      state = {
        ...state,
        revision: state.revision + 1,
        configRevision: state.configRevision + 1,
        mode: String(request.payload.mode) as "rule" | "global" | "direct",
      };
      changed = ["config", "status", "summary"];
    }
    if (request.command === "proxies.select") {
      const groupName = String(request.payload.group);
      const proxyName = String(request.payload.name);
      state = {
        ...state,
        revision: state.revision + 1,
        configRevision: state.configRevision + 1,
        proxyGroups: state.proxyGroups.map((group) =>
          group.name === groupName
            ? {
                ...group,
                selected: proxyName,
                proxies: group.proxies.map((proxy) => ({
                  ...proxy,
                  selected: proxy.name === proxyName,
                })),
              }
            : group,
        ),
      };
      changed = ["config", "summary", "proxies"];
    }
    host.reply(request, null, changed);
  };
  host.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

import { cleanup, host, mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest, DomainQueryResult } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  state = { ...state, vpnLifecycle: "connected", vpnRunning: true,
    running: true, networkProtected: true, vpnSessionId: "synthetic-session" };
  const server = new DomainHost(client, () => state);
  let hold = false;
  let pending: { request: CoreRequest; reply: DomainQueryResult } | undefined;
  server.onQuery = (request, reply) => {
    if (reply.domain !== "traffic" || !hold) return false;
    pending = { request, reply };
    host.post("paws.test.live.held", true);
    return true;
  };
  cleanup(host.on("paws.test.live.hold", () => {
    hold = true;
    state = { ...state, revision: state.revision + 1,
      traffic: { ...state.traffic, uploadSpeed: 2048, downloadSpeed: 65536 } };
    server.change(["traffic"]);
  }));
  cleanup(host.on("paws.test.live.release", () => {
    hold = false;
    if (pending) server.reply(pending.request, pending.reply);
    pending = undefined;
  }));
  cleanup(host.on("paws.test.live.fail", () => {
    hold = false;
    if (pending) server.result({ id: pending.request.id, ok: false, error: "Synthetic traffic query failure" });
    pending = undefined;
  }));
  cleanup(host.on("paws.test.live.tick", () => {
    state = { ...state, revision: state.revision + 1,
      connections: state.connections.map((row) => ({ ...row, downloadBytes: row.downloadBytes + 1024 })),
      providers: state.providers.map((row) => ({ ...row, members: row.members.map((member) => ({ ...member, delayMs: (member.delayMs || 0) + 1 })) })),
    };
    server.change(["connections", "providers"]);
  }));
  cleanup(host.on("paws.test.live.disconnect", () => {
    state = { ...state, revision: state.revision + 1,
      vpnRunning: false, vpnLifecycle: "stopped", networkProtected: false };
    server.change(["status"]);
  }));
  server.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

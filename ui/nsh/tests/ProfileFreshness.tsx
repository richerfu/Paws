import { cleanup, effect, host, mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import type { CoreRequest, DomainName, DomainQueryResult } from "../src/model";

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const server = new DomainHost(client, () => state);
  const held = new Map<DomainName, { request: CoreRequest; reply: DomainQueryResult }>();
  const holding = new Set<DomainName>();
  server.onQuery = (request, reply) => {
    if (!holding.has(reply.domain)) return false;
    held.set(reply.domain, { request, reply });
    return true;
  };
  server.onCommand = (request) => {
    host.post("paws.test.command", request.command);
    server.reply(request, null);
  };
  cleanup(
    host.on("paws.test.invalidate", (config: boolean) => {
      holding.add("profiles");
      if (config) holding.add("config");
      state = {
        ...state,
        revision: state.revision + 1,
        configRevision: state.configRevision + (config ? 1 : 0),
        profiles: state.profiles.map((profile) => ({
          ...profile,
          uploadBytes: profile.uploadBytes + 128,
        })),
      };
      server.change(config ? ["config", "profiles"] : ["profiles"]);
    }),
  );
  cleanup(
    host.on("paws.test.release", (domain: DomainName) => {
      holding.delete(domain);
      const pending = held.get(domain);
      held.delete(domain);
      if (pending) server.reply(pending.request, pending.reply);
    }),
  );
  cleanup(
    host.on("paws.test.fail-profiles", () => {
      const pending = held.get("profiles");
      held.delete("profiles");
      holding.delete("profiles");
      if (pending)
        server.result({
          id: pending.request.id,
          ok: false,
          error: "Synthetic profile query failure",
        });
    }),
  );
  cleanup(
    host.on("paws.test.attempt", () => {
      void client
        .send("profiles.subscription", {
          profileId: "test-profile",
          expectedRevision: state.configRevision,
          name: "Synthetic",
          url: "https://fixture.example/sub",
        })
        .catch(() => host.post("paws.test.blocked", true));
    }),
  );
  cleanup(host.on("paws.test.retry", () => client.domains.retry(["profiles"])));
  effect(() =>
    host.post("paws.test.readiness", {
      fresh: client.domains.fresh(["profiles"]),
      ready: client.domains.readyForCommand(["config", "profiles"]),
      config: client.domains.fresh(["config"]),
      configRevision: client.domains.configRevision(),
    }),
  );
  server.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

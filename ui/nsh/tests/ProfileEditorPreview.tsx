import { host as channel, mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import { testControl } from "./test-controls";

// Public synthetic YAML only. No application home, engine or VPN is opened.
mount(() => {
  const client = new CoreClient();
  let state = fixture(), content = "proxies: []\r\nrules: []\r\n";
  let failRead = false, failSave = false, failValidation = false, holdRead = false, holdSave = false;
  const reads: CoreRequest[] = [], saves: CoreRequest[] = [];
  const server = new DomainHost(client, () => state);
  const replySave = (request: CoreRequest) => {
    if (request.payload.expectedRevision !== state.configRevision) {
      server.result({ id: request.id, ok: false, error: "Synthetic revision conflict" });
      return;
    }
    content = String(request.payload.content);
    state = { ...state, revision: state.revision + 1, configRevision: state.configRevision + 1 };
    server.reply(request, null, ["config", "profiles"]);
  };
  server.onCommand = (request) => {
    channel.post("paws.test.report", { command: request.command, ...request.payload });
    if (request.command === "profiles.read") {
      if (failRead) {
        failRead = false;
        server.result({ id: request.id, ok: false, error: "Synthetic read failure" });
      } else if (holdRead) reads.push(request);
      else server.reply(request, content);
    } else if (request.command === "profiles.validate") {
      if (failValidation) server.result({ id: request.id, ok: false, error: "Synthetic YAML validation failure" });
      else server.reply(request, null);
    } else if (request.command === "profiles.save") {
      if (failSave) {
        failSave = false;
        server.result({ id: request.id, ok: false, error: "Synthetic save failure" });
      } else if (holdSave) saves.push(request);
      else replySave(request);
    } else throw new Error(`Unexpected editor fixture command: ${request.command}`);
  };
  const actions: Record<string, () => void> = {
    "fail-read": () => { failRead = true; },
    "fail-save": () => { failSave = true; },
    "fail-validation": () => { failValidation = true; },
    "hold-read": () => { holdRead = true; },
    "hold-save": () => { holdSave = true; },
    "release-read": () => { holdRead = false; for (const r of reads.splice(0)) server.reply(r, content); },
    "release-save": () => { holdSave = false; for (const r of saves.splice(0)) replySave(r); },
    "remote-change": () => {
      content = "proxies: []\nrules: []\n# refreshed remotely\n";
      state = { ...state, revision: state.revision + 1, configRevision: state.configRevision + 1 };
      server.change(["config", "profiles"]);
    },
    "large-document": () => { content = "proxies: []\nrules: []\n" + "# synthetic YAML 0123456789\n".repeat(4000); },
  };
  for (const id of Object.keys(actions)) testControl(`editor-fixture-${id}`, actions[id]);
  server.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

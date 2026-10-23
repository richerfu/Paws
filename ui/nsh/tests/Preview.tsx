import { mount } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { DomainHost } from "./DomainHost";
import type { DomainName } from "../src/model";
import { converterDraftFixture, fixture } from "./fixture";

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state);
  host.onCommand = (request) => {
    let changed: DomainName[] = [];
    const p = request.payload;
    const required: Record<string, string[]> = {
      "vpn.start": [],
      "vpn.stop": [],
      "vpn.restart": [],
      "mode.set": ["mode"],
      "profiles.read": ["profileId"],
      "profiles.validate": ["content"],
      "profiles.subscription": ["profileId", "name", "url", "expectedRevision"],
      "platform.qr": ["text"],
      "platform.exportQr": ["name", "pngBase64"],
      "rules.reorder": ["ids", "expectedRevision"],
      "providers.testProxy": ["provider", "name"],
      "proxies.select": ["group", "name"],
      "proxies.unfix": ["group"],
      "proxies.delay": ["name"],
      "proxies.testGroup": ["group"],
      "rules.add": [
        "matchKind",
        "value",
        "target",
        "expectedRevision",
        "connectionId",
        "disconnectAfterSave",
      ],
      "rules.lookup": ["query"],
      "profiles.cancelImport": ["intentId"],
    };
    if (
      required[request.command] &&
      Object.keys(p).sort().join(",") !== required[request.command].slice().sort().join(",")
    )
      throw new Error(`Wire contract mismatch for ${request.command}`);
    if (
      request.command === "profiles.subscription" &&
      (p.profileId !== "test-profile" || !Number.isInteger(p.expectedRevision))
    )
      throw new Error("Invalid subscription identity/revision");
    if (
      request.command === "rules.add" &&
      (p.value !== "api.example.com" ||
        p.connectionId !== "connection-1" ||
        p.disconnectAfterSave !== true ||
        p.matchKind !== "domain")
    )
      throw new Error("Activity rule lost match or connection context");
    if (request.command === "rules.reorder" && (p.ids as string[]).join(",") !== "rule-2,rule-1")
      throw new Error("Rule move sent the wrong full ordering");
    if (request.command === "providers.testProxy" && (!p.provider || !p.name))
      throw new Error("Provider member identity missing");
    console.log(`PASS WIRE: ${request.command}`);
    let data: unknown = null;
    if (request.command === "engines.decision")
      data = { selected: "meow", reasons: ["Fixture engine"] };
    if (request.command === "rules.add")
      data = {
        applied: { mutation: "inserted", liveUpdated: false, ruleModeActive: true },
        connectionCloseRequested: true,
        connectionCloseError: "Fixture close unavailable",
      };
    if (request.command === "logs.recordingStatus")
      data = { enabled: false, archives: [], lastError: "Fixture archive error" };
    if (request.command === "profiles.read") data = "proxies: []\nrules: []\n";
    if (request.command === "platform.qr")
      data = {
        source:
          "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6GxsAAAAASUVORK5CYII=",
        pngBase64:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6GxsAAAAASUVORK5CYII=",
      };
    if (
      request.command === "vpn.start" ||
      request.command === "vpn.restart" ||
      request.command === "vpn.stop"
    ) {
      const running = request.command !== "vpn.stop";
      state = {
        ...state,
        revision: state.revision + 1,
        vpnRunning: running,
        running,
        networkProtected: running,
        vpnLifecycle: running ? "connected" : "stopped",
        activeEngine: running ? "meow" : null,
      };
      changed = ["status", "summary"];
    }
    if (request.command === "proxies.select" || request.command === "proxies.unfix") {
      if (p.group !== "Proxy" || (request.command === "proxies.select" && p.name !== "Test node"))
        throw new Error("Proxy selection sent the wrong identity");
      state = {
        ...state,
        revision: state.revision + 1,
        configRevision: state.configRevision + 1,
        proxyGroups: state.proxyGroups.map((group) => ({
          ...group,
          fixed: request.command === "proxies.select" ? String(p.name) : "",
        })),
      };
      changed = ["config", "summary", "proxies"];
    }
    if (request.command === "preferences.set")
      client.preferences.set({ ...client.preferences(), ...request.payload });
    if (request.command === "converter.loadDraft") data = converterDraftFixture();
    if (request.command === "converter.buildUrl")
      data = "https://converter.example/sub?target=clash&url=https%3A%2F%2Ffixture.example%2Fsub";
    if (request.command === "rules.lookup")
      data = {
        query: p.query,
        inputKind: "domain",
        matched: true,
        target: "DIRECT",
        ruleLine: "DOMAIN-SUFFIX,example.com,DIRECT",
        resolutionAttempted: false,
      };
    if (request.command === "diagnostics.watchGet")
      data = { enabled: false, group: "Proxy", url: "https://www.gstatic.com/generate_204" };
    if (request.command === "proxies.testGroup")
      setTimeout(() => host.reply(request, data, changed), 15);
    else host.reply(request, data, changed);
  };
  host.connect();
  return <PawsApp client={client} embeddedGateway={false} />;
});

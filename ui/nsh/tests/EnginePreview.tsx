import { testControl } from "./test-controls";
import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest, DomainQueryResult, VpnOptions } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

/** Actual production client/page; only server receipts are controlled.
 * No Core, personal data, native engine or network is initialized. */
mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state);
  const shown = signal(true),
    tick = signal(0);
  const previews: CoreRequest[] = [],
    writes: CoreRequest[] = [];
  const queries: { request: CoreRequest; reply: DomainQueryResult }[] = [];
  let holdConfig = false,
    failSummary = false;
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  host.onQuery = (request, reply) => {
    reply.observation = { engine: "meow", session: state.vpnSessionId ?? null,
      issues: [{ field: "configuration", reason: "INTERNAL_CONFIGURATION_PREVIEW_METADATA" }] };
    if (failSummary && reply.domain === "summary") {
      host.result({ id: request.id, ok: false, error: "Actual summary query failure" });
      return true;
    }
    if (!holdConfig || reply.domain !== "config") return false;
    queries.push({ request, reply });
    return true;
  };
  host.onCommand = (request) => {
    if (request.command === "engines.decision") {
      previews.push(request);
      console.log(
        `PREVIEW RECEIVED: ${(request.payload.options as VpnOptions).engine} profile=${state.activeProfile} revision=${state.configRevision}`,
      );
    } else if (request.command === "settings.vpn") writes.push(request);
    else throw new Error(`Unexpected preview fixture command: ${request.command}`);
  };
  host.connect();
  const release = (last: boolean, fail = false, longReason = false, fallback = false) => {
    const request = last ? previews.pop() : previews.shift();
    assert(request, "No held engine preview");
    if (fail) host.result({ id: request!.id, ok: false, error: "Controlled preview failure" });
    else {
      const engine = (request!.payload.options as VpnOptions).engine;
      const selected = fallback ? "mihomo" : engine === "auto" ? "meow" : engine === "singbox" ? "sing-box" : engine;
      host.reply(request!, { selected, reasons: [longReason
        ? "Long compatibility explanation " + "x".repeat(200)
        : fallback ? "Requested engine cannot compile this public synthetic rule" : `reason-${engine}`] });
    }
  };
  const change = (profile: boolean) => {
    state = {
      ...state,
      revision: state.revision + 1,
      configRevision: state.configRevision + 1,
      ...(profile ? { activeProfile: "other-public-profile" } : {}),
    };
    host.change(["config"]);
  };
  const checkpoint = (name: string) => {
    if (name === "nonblocking")
      assert(
        previews.length > 0 && client.isPending("engines.decision") && client.pending() === 0,
        "Readonly decision entered global write pending or lost receipt tracking",
      );
    if (name === "write")
      assert(
        writes.length === 1 && client.pending() === 1,
        "Actual write was not globally pending",
      );
    if (name === "barrier")
      assert(
        writes.length === 0 && queries.length > 0 && client.pending() === 1,
        "Actual write escaped its query barrier",
      );
    if (name === "idle")
      assert(
        client.pending() === 0 && client.domains.fresh(["config"]),
        "Write did not complete after authoritative query",
      );
    if (name === "error-local")
      assert(
        client.error() === "" && client.pending() === 0,
        "Preview failure escaped page-local feedback",
      );
    if (name === "same-option")
      assert(
        previews.length === 1 && writes.length === 0 && client.pending() === 0,
        "Reselecting the current engine dispatched a preview/write",
      );
    if (name === "metadata-hidden")
      assert(client.domains.issues(["config"]).includes("INTERNAL_CONFIGURATION_PREVIEW_METADATA"),
        "Observation metadata was deleted rather than merely hidden");
    if (name === "query-error")
      assert(client.domains.error(["summary"]) === "Actual summary query failure" && writes.length === 0,
        "Actual query failure was lost or replayed a write");
    if (name === "query-recovered")
      assert(client.domains.error(["summary"]) === "" && client.domains.fresh(["summary"]) && writes.length === 0,
        "Readonly Retry did not recover actual query failure");
    console.log(`PASS PREVIEW: ${name}`);
  };
  const port = (id: string, action: () => void) => {
    const press = testControl(`preview-${id}`, () => { action(); tick.set(tick() + 1); });
    return <View testID={`preview-${id}`} onPress={press}><Text>{id}</Text></View>;
  };
  return (
    <View style={{ flex: 1 }}>
      <If when={shown()}>
        <PawsApp client={client} embeddedGateway={false} />
      </If>
      <View style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0 }}>
        <Text>{tick()}</Text>
        {port("release-first", () => release(false))}
        {port("release-long", () => release(false, false, true))}
        {port("release-fallback", () => release(false, false, false, true))}
        {port("release-last", () => release(true))}
        {port("fail-first", () => release(false, true))}
        {port("revision", () => change(false))}
        {port("profile", () => change(true))}
        {port("english", () => client.preferences.set({ ...client.preferences(), language: "en" }))}
        {port("query-error", () => {
          failSummary = true;
          state = { ...state, revision: state.revision + 1 };
          host.change(["summary"]);
        })}
        {port("query-allow", () => { failSummary = false; })}
        {port("write-result", () => {
          assert(writes.length === 1, "No write to release");
          const request = writes.shift()!;
          holdConfig = true;
          state = {
            ...state,
            revision: state.revision + 1,
            configRevision: state.configRevision + 1,
            vpnOptions: request.payload.options as VpnOptions,
          };
          host.reply(request, true, ["config"]);
        })}
        {port("query-result", () => {
          holdConfig = false;
          for (const value of queries.splice(0)) host.reply(value.request, value.reply);
        })}
        {port("hide", () => shown.set(false))}
        {port("show", () => shown.set(true))}
        {port("assert-nonblocking", () => checkpoint("nonblocking"))}
        {port("assert-write", () => checkpoint("write"))}
        {port("assert-barrier", () => checkpoint("barrier"))}
        {port("assert-idle", () => checkpoint("idle"))}
        {port("assert-error-local", () => checkpoint("error-local"))}
        {port("assert-same-option", () => checkpoint("same-option"))}
        {port("assert-metadata-hidden", () => checkpoint("metadata-hidden"))}
        {port("assert-query-error", () => checkpoint("query-error"))}
        {port("assert-query-recovered", () => checkpoint("query-recovered"))}
      </View>
    </View>
  );
});

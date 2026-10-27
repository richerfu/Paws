import { testControl } from "./test-controls";
import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest, DomainQueryResult, VpnOptions } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

/** The runner registers the real Field's Input.onChangeText in an owned copy.
 * This exercises the production handler/client, not native typing or IME. */
type InputPort = { change: (value: string) => void; editable: () => boolean };
const inputHost = globalThis as typeof globalThis & { __pawsSettingsMtuInput?: InputPort };

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state);
  const tick = signal(0);
  const writes: CoreRequest[] = [];
  const queries: { request: CoreRequest; reply: DomainQueryResult }[] = [];
  let holdConfig = false;
  let received = 0;
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  host.onQuery = (request, reply) => {
    if (!holdConfig || reply.domain !== "config") return false;
    queries.push({ request, reply });
    return true;
  };
  host.onCommand = (request) => {
    if (request.command === "engines.decision") {
      host.reply(request, { selected: "meow", reasons: [] });
      return;
    }
    assert(request.command === "settings.vpn", `Unexpected MTU command: ${request.command}`);
    assert(request.payload.profileId === "test-profile", "MTU write lost the profile owner");
    assert(request.payload.expectedRevision === state.configRevision, "MTU write lost its revision");
    assert((request.payload.options as VpnOptions).mtu === 2816, "MTU write did not contain numeric 2816");
    received++;
    writes.push(request);
    console.log("PASS MTU: actual settings.vpn payload mtu=2816 profile/revision preserved");
  };
  host.connect();
  const input = (value: string) => {
    const port = inputHost.__pawsSettingsMtuInput;
    assert(port, "Real Field Input.onChangeText was not registered by the owned runner");
    assert(port!.editable(), "Attempt to inject a change into a disabled Field");
    port!.change(value);
    console.log(`MTU HANDLER: ${JSON.stringify(value)}`);
  };
  const check = (phase: string) => {
    if (phase === "no-write")
      assert(received === 0 && client.pending() === 0, "Input alone dispatched a settings write");
    if (phase === "write")
      assert(received === 1 && writes.length === 1 && client.pending() === 1, "MTU write was not held/pending");
    if (phase === "barrier")
      assert(writes.length === 0 && queries.length > 0 && client.pending() === 1, "MTU write escaped the config barrier");
    if (phase === "saved")
      assert(received === 1 && client.pending() === 0 && client.domains.fresh(["config"]) &&
        state.vpnOptions.mtu === 2816, "MTU save did not finish with authoritative 2816");
    console.log(`PASS MTU: ${phase}`);
  };
  const port = (id: string, action: () => void) => {
    const press = testControl(`mtu-${id}`, () => { action(); tick.set(tick() + 1); });
    return <View testID={`mtu-${id}`} onPress={press}><Text>{id}</Text></View>;
  };
  return (
    <View style={{ flex: 1 }}>
      <PawsApp client={client} embeddedGateway={false} />
      <View style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0 }}>
        <Text>{tick()}</Text>
        {port("same-initial", () => input("1500"))}
        {port("2816", () => input("2816"))}
        {port("legacy-minimum", () => input("576"))}
        {port("below", () => input("1279"))}
        {port("above", () => input("65536"))}
        {port("fraction", () => input("1500.5"))}
        {port("empty", () => input(""))}
        {port("minimum", () => input("1280"))}
        {port("maximum", () => input("65535"))}
        {port("assert-no-write", () => check("no-write"))}
        {port("assert-write", () => check("write"))}
        {port("assert-barrier", () => check("barrier"))}
        {port("assert-saved", () => check("saved"))}
        {port("write-result", () => {
          assert(writes.length === 1, "No MTU write to release");
          const request = writes.shift()!;
          holdConfig = true;
          state = { ...state, revision: state.revision + 1, configRevision: state.configRevision + 1,
            vpnOptions: request.payload.options as VpnOptions };
          host.reply(request, true, ["config"]);
        })}
        {port("query-result", () => {
          holdConfig = false;
          for (const value of queries.splice(0)) host.reply(value.request, value.reply);
        })}
      </View>
    </View>
  );
});

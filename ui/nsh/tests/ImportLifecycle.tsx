import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { CoreRequest, DomainName, DomainQueryResult } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import { testControl } from "./test-controls";

/** Exercises the actual header, import Dialog and command gates. Held reads
 * represent a cached page revalidating; no profile is persisted or imported. */
mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state),
    tick = signal(0);
  const held = new Set<DomainName>();
  const replies: { request: CoreRequest; reply: DomainQueryResult }[] = [];
  host.onQuery = (request, reply) => {
    if (!held.has(reply.domain)) return false;
    replies.push({ request, reply });
    return true;
  };
  host.onCommand = (request) => {
    if (request.command === "platform.scan") host.reply(request, "https://fixture.example/sub");
    else if (request.command === "profiles.parseScan")
      host.reply(request, { url: "https://fixture.example/sub" });
    else if (request.command === "profiles.cancelImport") host.reply(request, true);
    else throw new Error(`Unexpected mutation: ${request.command}`);
  };
  host.connect();
  const dirty = (domain: "profiles" | "config") => {
    held.add(domain);
    state = { ...state, revision: state.revision + 1 };
    host.change([domain]);
    if (client.domains.fresh([domain])) throw new Error("Held domain was incorrectly fresh");
    tick.set(tick() + 1);
    console.log(`PASS: cached ${domain} remains visible while its query is pending`);
  };
  const blocked = async () => {
    let rejected = false;
    try {
      await client.send("profiles.importUrl", { url: "https://fixture.example/sub" });
    } catch {
      rejected = true;
    }
    if (
      !rejected ||
      client.domains.fresh(["config"]) ||
      host.seen.some(
        (request) =>
          request.command === "profiles.importUrl" || request.command === "profiles.importContent",
      )
    )
      throw new Error("Import bypassed its stale config gate");
    tick.set(tick() + 1);
    console.log("PASS: opening import does not bypass config freshness at submission");
  };
  const release = () => {
    held.clear();
    for (const { request, reply } of replies.splice(0)) host.reply(request, reply);
    tick.set(tick() + 1);
  };
  const ready = () => {
    if (!client.domains.fresh(["config", "profiles"]))
      throw new Error("Released reads did not settle");
    tick.set(tick() + 1);
    console.log("PASS: import Dialog close and reopen preserve its page owner and pending reads");
  };
  const assertBlocked = testControl("assert-import-blocked", blocked);
  const holdProfiles = testControl("hold-profiles", () => dirty("profiles"));
  const holdConfig = testControl("hold-config", () => dirty("config"));
  const releaseReads = testControl("release-import-reads", release);
  const assertReady = testControl("assert-import-ready", ready);
  return (
    <>
      <PawsApp client={client} embeddedGateway={false} />
      <View style={{ position: "absolute", width: 1, height: 1, left: 0, top: 0 }}>
        <View testID="hold-profiles" onPress={holdProfiles}>
          <Text>{tick()}</Text>
        </View>
        <View testID="hold-config" onPress={holdConfig}>
          <Text>Hold config</Text>
        </View>
        <View testID="assert-import-blocked" onPress={assertBlocked}>
          <Text>Assert write gate</Text>
        </View>
        <View testID="release-import-reads" onPress={releaseReads}>
          <Text>Release reads</Text>
        </View>
        <View testID="assert-import-ready" onPress={assertReady}>
          <Text>Assert read completion</Text>
        </View>
      </View>
    </>
  );
});

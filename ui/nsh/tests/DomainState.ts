import { computed, effect, untracked } from "nsh";
import { CoreClient } from "../src/client";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import type { CoreRequest, DomainQueryResult, StateChanged } from "../src/model";

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function turn() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  for (let index = 0; index < 8; index++) await Promise.resolve();
}

/** Actual production client, real request promises and explicit native metadata.
 * The fake server supplies data but contains no client merge/interest reducer. */
export async function domainStateTrace(): Promise<void> {
  const client = new CoreClient(1000, 100),
    first = fixture();
  let state = first;
  const host = new DomainHost(client, () => state);
  let invalidateAfterApply = false;
  effect(() => {
    const traffic = client.domains.data("traffic");
    if (invalidateAfterApply && traffic?.traffic.downloadSpeed === 60) {
      invalidateAfterApply = false;
      untracked(() => {
        state = {
          ...state,
          revision: state.revision + 1,
          traffic: { ...state.traffic, downloadSpeed: 70 },
        };
        host.change(["traffic"]);
      });
    }
  });
  let initial: { request: CoreRequest; change: StateChanged } | undefined;
  host.onSubscribe = (request, change) => {
    if (!initial) {
      initial = { request, change };
      return true;
    }
    return false;
  };
  client.domains.setPage(["summary"]);
  host.connect();
  try {
    await turn();
    assert(
      initial &&
        host.seen.length === 1 &&
        host.seen[0].command === "state.subscribe" &&
        !client.state(),
      "queries ran before subscription registration",
    );
    host.reply(initial.request, initial.change);
    await turn();
    assert(
      client.state() && client.domains.has(["summary"]),
      "critical and initial requested domains did not load",
    );
    assert(
      host.seen
        .filter((request) => request.command === "state.query")
        .map((request) => request.payload.domain)
        .sort()
        .join(",") === "config,status,summary",
      "Home queried a full roster or an unrequested domain",
    );
    assert(
      client.state()!.rules === undefined &&
        client.state()!.proxyGroups === undefined &&
        client.state()!.connections === undefined,
      "missing detail domains masqueraded as loaded empty values",
    );
    const home = client.domains.data("summary")!,
      options = client.state()!.vpnOptions;
    let summaryRuns = 0;
    const headers = computed(() => {
      summaryRuns++;
      return client.domains.data("summary")!.groups;
    });
    headers();
    state = {
      ...state,
      revision: state.revision + 1,
      traffic: { ...state.traffic, downloadSpeed: 8192 },
    };
    host.change(["summary", "traffic"]);
    await turn();
    assert(
      client.domains.data("summary") === home && headers() === home.groups && summaryRuns === 1,
      "unchanged cheap summary rebuilt group headers",
    );
    assert(
      client.state()!.vpnOptions === options && !client.domains.has(["traffic"]),
      "unrequested telemetry changed static configuration or fabricated traffic data",
    );

    client.domains.setPage(["profiles"]);
    await turn();
    const profiles = client.state()!.profiles;
    state = {
      ...state,
      profiles: state.profiles.map((profile) => ({ ...profile, uploadBytes: 8192 })),
    };
    host.change(["profiles"]); // Same revision is a duplicate and must stay stable.
    state = { ...state, revision: state.revision + 1 };
    host.change(["profiles"]);
    await turn();
    assert(
      client.state()!.profiles === profiles && profiles[0].uploadBytes === 8192,
      "profile usage query rebuilt list identities or lost counters",
    );

    client.domains.setPage(["traffic"]);
    await turn();
    const count = host.seen.length;
    state = { ...state, revision: state.revision + 1, configRevision: 2, mode: "direct" };
    host.change(["config", "summary"]);
    await turn();
    assert(
      client.state()!.configRevision === 2 && client.state()!.mode === "direct",
      "critical config did not advance independently",
    );
    assert(
      !host.seen.slice(count).some((request) => request.payload.domain === "summary"),
      "departed Home kept its summary query stream",
    );
    assert(
      client.domains.data("traffic")!.connectionSummary.recent.length <= 5 &&
        client.state()!.connections === undefined,
      "Traffic fetched the full connections list",
    );

    const releaseGroup = client.domains.acquire([{ domain: "proxies", groupKey: "Proxy" }]);
    await turn();
    const groupRequest = host.seen.find(
      (request) => request.command === "state.query" && request.payload.groupKey === "Proxy",
    );
    assert(
      groupRequest &&
        client.domains.data("proxies", "Proxy")!.proxyGroups.length === 1 &&
        client.state()!.proxyGroups === undefined,
      "expanded group fetched or replaced the full roster",
    );
    releaseGroup();
    await turn();
    const collapsed = host.seen.length;
    host.change(["proxies"]);
    await turn();
    assert(host.seen.length === collapsed, "collapsed group remained subscribed");

    let held: { request: CoreRequest; reply: DomainQueryResult } | undefined;
    host.onQuery = (request, reply) => {
      if (reply.domain === "traffic" && !held) {
        held = { request, reply };
        return true;
      }
      return false;
    };
    state = {
      ...state,
      revision: state.revision + 1,
      traffic: { ...state.traffic, downloadSpeed: 40 },
    };
    host.change(["traffic"]);
    await turn();
    assert(held, "traffic query was not sent");
    const inflight = host.seen.filter(
      (request) => request.command === "state.query" && request.payload.domain === "traffic",
    ).length;
    state = {
      ...state,
      revision: state.revision + 1,
      traffic: { ...state.traffic, downloadSpeed: 50 },
    };
    host.change(["traffic"]);
    host.change(["traffic"]);
    await turn();
    assert(
      host.seen.filter(
        (request) => request.command === "state.query" && request.payload.domain === "traffic",
      ).length === inflight,
      "same-key invalidations started overlapping queries",
    );
    host.reply(held.request, held.reply);
    await turn();
    assert(
      client.state()!.traffic.downloadSpeed === 50,
      "late query overwrote a newer target instead of querying again",
    );
    assert(
      host.seen.filter(
        (request) => request.command === "state.query" && request.payload.domain === "traffic",
      ).length ===
        inflight + 1,
      "targets did not coalesce into one follow-up query",
    );
    host.onQuery = undefined;
    invalidateAfterApply = true;
    state = {
      ...state,
      revision: state.revision + 1,
      traffic: { ...state.traffic, downloadSpeed: 60 },
    };
    host.change(["traffic"]);
    await turn();
    assert(
      client.state()!.traffic.downloadSpeed === 70 && client.domains.fresh(["traffic"]),
      "invalidation between query apply and flight finalization was lost",
    );

    let old: { request: CoreRequest; reply: DomainQueryResult } | undefined;
    host.onQuery = (request, reply) => {
      if (reply.groupKey === "Old") {
        old = { request, reply };
        return true;
      }
      return false;
    };
    const releaseOld = client.domains.acquire([{ domain: "proxies", groupKey: "Old" }]);
    await turn();
    assert(old, "old-generation query was not held");
    releaseOld();
    client.domains.setPage(["about"]);
    await turn();
    host.reply(old.request, old.reply);
    await turn();
    assert(
      !client.domains.data("proxies", "Old") && client.domains.has(["about"]),
      "old generation/session response mutated a departed domain",
    );

    host.onQuery = (_request, reply) => {
      if (reply.domain === "dns") reply.data = {} as typeof reply.data;
      return false;
    };
    client.domains.setPage(["dns"]);
    await turn();
    assert(
      !client.domains.has(["dns"]) && client.domains.error(["dns"]).includes("missing dns"),
      "missing domain field became a valid empty page",
    );
    host.onQuery = undefined;
    client.domains.retry(["dns"]);
    await turn();
    assert(
      client.domains.fresh(["dns"]) && !client.domains.error(["dns"]),
      "explicit read retry did not recover an errored domain",
    );

    client.domains.setPage(["rules"]);
    await turn();
    let commits = 0;
    const receipt = {
      applied: { mutation: "inserted", liveUpdated: false, ruleModeActive: true },
      connectionCloseRequested: true,
      connectionCloseError: "Close unavailable",
    };
    host.onCommand = (request) => {
      commits++;
      state = { ...state, revision: state.revision + 1, configRevision: state.configRevision + 1 };
      host.reply(request, receipt, ["config", "rules"]);
    };
    const heldSave: { request: CoreRequest; reply: DomainQueryResult }[] = [];
    host.onQuery = (request, reply) => {
      if (reply.domain === "rules") {
        heldSave.push({ request, reply });
        return true;
      }
      return false;
    };
    let completed = false;
    const save = client.run<typeof receipt>("rules.add", {}, "Saved").then((data) => {
      completed = true;
      return data;
    });
    await turn();
    assert(
      heldSave.length === 1 && !completed && client.pending() === 1,
      "success resolved or released its busy gate before the receipt domain revision",
    );
    const beforeBlockedWrite = host.seen.length;
    assert(
      (await client.run("rules.add")) === undefined && host.seen.length === beforeBlockedWrite,
      "cached stale data allowed another mutation while its query was pending",
    );
    host.reply(heldSave[0].request, heldSave[0].reply);
    await turn();
    assert(
      (await save)?.connectionCloseError === "Close unavailable" &&
        Number(commits) === 1 &&
        client.notice() === "Saved",
      "barrier lost actual result data or replayed the mutation",
    );

    host.onQuery = undefined;
    host.onCommand = (request) => {
      commits++;
      host.result({
        id: request.id,
        ok: true,
        data: receipt,
        stateRefreshError: "metadata unavailable",
      });
    };
    const committed = await client.run<typeof receipt>("rules.add", {}, "Incorrect success");
    await turn();
    assert(
      committed?.connectionCloseError === "Close unavailable" &&
        client.warning().includes("Changes were applied") &&
        client.notice() === "" &&
        Number(commits) === 2,
      "committed synchronization failure lost data, claimed normal success or repeated the mutation",
    );

    host.onCommand = (request) => {
      commits++;
      state = { ...state, revision: state.revision + 1 };
      host.reply(request, receipt, ["rules"]);
    };
    host.onQuery = (request, reply) => {
      if (reply.domain === "rules") {
        host.result({ id: request.id, ok: false, error: "Query failed" });
        return true;
      }
      return false;
    };
    const queryFailure = await client.run<typeof receipt>("rules.add", {}, "Incorrect success");
    assert(
      queryFailure?.connectionCloseError === "Close unavailable" &&
        client.warning().includes("Query failed") &&
        client.notice() === "" &&
        Number(commits) === 3,
      "failed synchronization discarded a committed result or hung its receipt",
    );
    const failedReadRequests = host.seen.length;
    assert(
      (await client.run("rules.add")) === undefined && host.seen.length === failedReadRequests,
      "query error left a cached list writable",
    );
    host.onQuery = undefined;
    client.domains.retry(["rules"]);
    await turn();
    assert(
      Number(commits) === 3 && client.domains.fresh(["rules"]),
      "read retry re-executed a mutation",
    );

    const departed: { request: CoreRequest; reply: DomainQueryResult }[] = [];
    host.onQuery = (request, reply) => {
      if (reply.domain === "rules") {
        departed.push({ request, reply });
        return true;
      }
      return false;
    };
    const leaving = client.run<typeof receipt>("rules.add");
    await turn();
    assert(departed.length === 1, "departing page barrier had no pending query");
    client.domains.setPage(["about"]);
    await turn();
    assert(
      (await leaving)?.applied.mutation === "inserted" && Number(commits) === 4,
      "unsubscribe held a committed receipt or replayed it",
    );
    const currentConfig = client.state()!.configRevision;
    host.reply(departed[0].request, departed[0].reply);
    await turn();
    assert(
      client.state()!.configRevision === currentConfig && client.domains.has(["about"]),
      "late departed response changed current state",
    );
    console.log(
      "PASS: query interests, generations, coalescing, loading and committed-result barriers preserve state without aggregate pushes",
    );
  } finally {
    client.dispose();
  }
}

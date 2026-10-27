import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { Action, IconButton } from "../src/components";
import { CoreClient } from "../src/client";
import type { CoreRequest, DomainName, DomainQueryResult } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import { testControl } from "./test-controls";

/** Actual production controls/pages, synthetic server receipts only. No network,
 * Core, profile persistence or platform services are initialized by this entry. */
mount(() => {
  const client = new CoreClient();
  let state = fixture();
  state = {
    ...state,
    providers: [state.providers[0], { ...state.providers[0], name: "Second provider" }],
  };
  const host = new DomainHost(client, () => state);
  const shown = signal(true),
    controls = signal(true),
    actionBusy = signal(false),
    iconBusy = signal(false),
    tick = signal(0);
  let actionCalls = 0,
    iconCalls = 0,
    holdQueries = false,
    holdConfig = false,
    latestKey = "";
  const calls = new Map<string, number>();
  const pending: CoreRequest[] = [];
  const queries: { request: CoreRequest; reply: DomainQueryResult }[] = [];
  const key = (request: CoreRequest) =>
    `${request.command}:${request.payload.profileId || request.payload.name || ""}:${request.payload.providerType || ""}`;
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  host.onQuery = (request, reply) => {
    if (
      !(holdQueries && ["profiles", "providers"].includes(reply.domain)) &&
      !(holdConfig && reply.domain === "config")
    )
      return false;
    queries.push({ request, reply });
    return true;
  };
  host.onCommand = (request) => {
    if (request.command === "diagnostics.watchGet") {
      host.reply(request, { enabled: false, group: "Proxy", url: "https://fixture.example/check" });
      return;
    }
    if (
      ![
        "profiles.refreshAll",
        "profiles.refresh",
        "providers.refresh",
        "providers.healthcheck",
        "diagnostics.healthGet",
        "diagnostics.report",
      ].includes(request.command)
    ) {
      throw new Error(`Refresh fixture received unexpected command: ${request.command}`);
    }
    latestKey = key(request);
    calls.set(latestKey, (calls.get(latestKey) || 0) + 1);
    pending.push(request);
    tick.set(tick() + 1);
    console.log(`REFRESH RECEIVED: ${latestKey}`);
  };
  host.connect();
  const release = (fail: boolean) => {
    assert(pending.length === 1, "Expected one actual operation receipt to release");
    const request = pending.shift()!;
    if (fail) {
      host.result({ id: request.id, ok: false, error: "Controlled refresh failure" });
      return;
    }
    const changes: DomainName[] = request.command.startsWith("profiles.")
      ? ["config", "profiles"]
      : request.command.startsWith("providers.")
        ? ["providers"]
        : [];
    if (changes.length) state = { ...state, revision: state.revision + 1 };
    const data =
      request.command === "diagnostics.report"
        ? "Synthetic report without personal data"
        : request.command === "diagnostics.healthGet"
          ? null
          : request.command === "profiles.refreshAll"
            ? ["test-profile"]
            : true;
    host.reply(request, data, changes);
  };
  const checkpoint = (name: string) => {
    if (name === "shared-one")
      assert(
        actionCalls === 1 && iconCalls === 1 && actionBusy() && iconBusy(),
        "Loading controls dispatched a duplicate press",
      );
    if (name === "one")
      assert(
        pending.length === 1 && client.pending() === 1,
        "Refresh duplicate or missing pending operation",
      );
    if (name === "barrier")
      assert(
        pending.length === 0 && queries.length > 0 && client.pending() === 1,
        "Receipt released pending before the actual query barrier",
      );
    if (name === "idle")
      assert(
        pending.length === 0 && queries.length === 0 && client.pending() === 0,
        "Refresh did not reset after its final receipt/barrier",
      );
    if (name === "error")
      assert(
        client.error().includes("Controlled refresh failure") && client.pending() === 0,
        "Failed refresh did not reset/report failure",
      );
    if (name === "warning")
      assert(
        client.warning().includes("Controlled refresh synchronization") &&
          client.notice() === "" &&
          client.pending() === 0,
        "Committed refresh warning was overwritten by a success summary",
      );
    if (name === "config-held")
      assert(
        !client.domains.fresh(["config"]) &&
          !host.seen.some((request) => request.command === "profiles.refreshAll"),
        "Stale config admitted refreshAll or was incorrectly fresh",
      );
    if (name === "health-initial")
      assert(
        client.pending() === 0 && client.notice() === "",
        "Initial health read emitted manual-refresh feedback",
      );
    if (name === "health-refreshed")
      assert(
        client.pending() === 0 && client.notice() === "检测状态已刷新",
        "Manual health refresh did not report its actual completion",
      );
    tick.set(tick() + 1);
    console.log(`PASS REFRESH: ${name} latest=${latestKey} calls=${calls.get(latestKey) || 0}`);
  };
  for (const [id, name] of Object.entries({
    shared: "shared-one", one: "one", barrier: "barrier", idle: "idle",
    error: "error", warning: "warning", "config-held": "config-held",
    "health-initial": "health-initial", "health-refreshed": "health-refreshed",
  })) testControl(`refresh-assert-${id}`, () => checkpoint(name));
  const releaseWarning = testControl("refresh-release-warning", () => {
    assert(pending.length === 1, "No refresh receipt for warning");
    const request = pending.shift()!;
    host.result({ id: request.id, ok: true, data: ["test-profile"],
      stateRefreshError: "Controlled refresh synchronization warning" });
  });
  const holdConfigRead = testControl("refresh-hold-config", () => {
    holdConfig = true;
    state = { ...state, revision: state.revision + 1 };
    host.change(["config"]);
    tick.set(tick() + 1);
  });
  const hideApp = testControl("refresh-hide-app", () => shown.set(false));
  return (
    <View style={{ flex: 1 }}>
      <View style={{ flex: 1 }}>
        <If when={shown()}>
          <PawsApp client={client} embeddedGateway={false} />
        </If>
      </View>
      <View style={{ height: 92, paddingHorizontal: 4, gap: 2, backgroundColor: "#f4f4f5" }}>
        <Text
          style={{ fontSize: 10 }}
          testID="refresh-status"
        >{`${tick()} pending=${client.pending()} held=${pending.length} query=${queries.length} ${latestKey} calls=${calls.get(latestKey) || 0}`}</Text>
        <If
          when={controls()}
          else={
            <View style={{ flexDirection: "row", gap: 4 }}>
              <Action
                id="refresh-hold-query"
                small
                label="Hold query"
                onPress={() => {
                  holdQueries = true;
                  tick.set(tick() + 1);
                }}
              />
              <Action
                id="refresh-release-success"
                small
                label="Success"
                onPress={() => release(false)}
              />
              <Action
                id="refresh-release-failure"
                small
                label="Fail"
                onPress={() => release(true)}
              />
              <Action
                id="refresh-release-query"
                small
                label="Query ready"
                onPress={() => {
                  holdQueries = false;
                  holdConfig = false;
                  for (const value of queries.splice(0)) host.reply(value.request, value.reply);
                }}
              />
            </View>
          }
        >
          <View style={{ flexDirection: "row", gap: 4 }}>
            <Action
              id="refresh-shared-action"
              label="Shared refresh"
              icon="refresh-cw"
              loading={actionBusy()}
              onPress={() => {
                actionCalls++;
                actionBusy.set(true);
                console.log(`REFRESH SHARED: action=${actionCalls}`);
              }}
            />
            <IconButton
              id="refresh-shared-icon"
              label="Shared refresh icon"
              icon="refresh-cw"
              loading={iconBusy()}
              onPress={() => {
                iconCalls++;
                iconBusy.set(true);
                console.log(`REFRESH SHARED: icon=${iconCalls}`);
              }}
            />
            <Action
              id="refresh-hide-controls"
              small
              label="Page tests"
              onPress={() => controls.set(false)}
            />
          </View>
        </If>
      </View>
      <View style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0 }}>
        <View testID="refresh-assert-shared" onPress={() => checkpoint("shared-one")}>
          <Text>Shared loading guard</Text>
        </View>
        <View testID="refresh-assert-one" onPress={() => checkpoint("one")}>
          <Text>Assert single dispatch</Text>
        </View>
        <View testID="refresh-assert-barrier" onPress={() => checkpoint("barrier")}>
          <Text>Assert barrier pending</Text>
        </View>
        <View testID="refresh-assert-idle" onPress={() => checkpoint("idle")}>
          <Text>Assert idle</Text>
        </View>
        <View testID="refresh-assert-error" onPress={() => checkpoint("error")}>
          <Text>Assert failed reset</Text>
        </View>
        <View
          testID="refresh-release-warning"
          onPress={releaseWarning}
        >
          <Text>Committed warning</Text>
        </View>
        <View testID="refresh-assert-warning" onPress={() => checkpoint("warning")}>
          <Text>Assert synchronization warning</Text>
        </View>
        <View
          testID="refresh-hold-config"
          onPress={holdConfigRead}
        >
          <Text>Hold config freshness</Text>
        </View>
        <View testID="refresh-assert-config-held" onPress={() => checkpoint("config-held")}>
          <Text>Assert stale config gate</Text>
        </View>
        <View testID="refresh-assert-health-initial" onPress={() => checkpoint("health-initial")}>
          <Text>Initial read is silent</Text>
        </View>
        <View
          testID="refresh-assert-health-refreshed"
          onPress={() => checkpoint("health-refreshed")}
        >
          <Text>Manual refresh feedback</Text>
        </View>
        <View testID="refresh-hide-app" onPress={hideApp}>
          <Text>Unmount pending page</Text>
        </View>
      </View>
    </View>
  );
});

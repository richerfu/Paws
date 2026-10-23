import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { errorTextRows } from "../src/page-error";
import type { UiState } from "../src/model";
import { DomainHost } from "./DomainHost";
import { converterDraftFixture, fixture } from "./fixture";

/** Public synthetic data only; real Paws components, no Core/network/VPN. */
export const VISUAL_NAME =
  "SyntheticSubscription_UnbrokenABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_完整公开合成名称😀_ABCDEFGHIJKLMNOPQRSTUVWXYZ";
export const VISUAL_URL =
  "https://visual-fixture.example/public/" +
  "long-synthetic-path-segment/".repeat(5) +
  "fixture.yaml";
export const VISUAL_ERROR =
  "Synthetic error: the public fixture intentionally reports a failed cache refresh; 这不是个人订阅或真实网络故障。";
export const VISUAL_REFRESH_ERROR = (VISUAL_ERROR + "\n").repeat(12);

export function visualFixture(): UiState {
  const base = fixture();
  const connection = base.connections[0];
  return {
    ...base,
    vpnSessionId: "visual-current-session",
    vpnLifecycle: "connected",
    engineLoaded: true,
    activeEngine: "meow",
    running: true,
    vpnRunning: true,
    networkProtected: false,
    networkProtectError: VISUAL_ERROR,
    activeProfile: "visual-active",
    controllerAddr: "127.0.0.1:9090",
    profiles: [
      {
        ...base.profiles[0],
        id: "visual-active",
        name: VISUAL_NAME,
        active: true,
        subscriptionUrl: VISUAL_URL,
        lastRefreshAt: "2026-10-22T00:00:00Z",
      },
      {
        ...base.profiles[0],
        id: "visual-error",
        name: "公开合成失败订阅 — " + VISUAL_NAME,
        active: false,
        subscriptionUrl: VISUAL_URL,
        lastRefreshError: VISUAL_REFRESH_ERROR,
      },
    ],
    rules: [
      {
        ...base.rules[0],
        line:
          "DOMAIN-SUFFIX," +
          "long-public-domain-segment.".repeat(12) +
          "visual-fixture.example,DIRECT",
      },
    ],
    proxyGroups: [
      {
        ...base.proxyGroups[0],
        name: "Synthetic group — " + VISUAL_NAME,
        selected: "Synthetic node — " + VISUAL_NAME,
        proxies: [{ ...base.proxyGroups[0].proxies[0], name: "Synthetic node — " + VISUAL_NAME }],
      },
    ],
    providers: [
      {
        ...base.providers[0],
        name: "Synthetic cached provider — " + VISUAL_NAME,
        path: "/public/fixture/" + "very-long-segment/".repeat(4) + "provider.yaml",
        url: VISUAL_URL,
        cacheExists: true,
      },
      {
        ...base.providers[0],
        name: "Synthetic failed provider",
        cacheExists: true,
        lastRefreshError: VISUAL_ERROR,
        url: VISUAL_URL,
      },
      {
        ...base.providers[0],
        name: "Synthetic pending provider",
        cacheExists: false,
        cacheBytes: 0,
      },
    ],
    connections: [
      {
        ...connection,
        id: "visual-live",
        domain:
          "a-very-long-public-synthetic-host-name-without-short-segments.visual-fixture.example",
        proxy: "Synthetic route — " + VISUAL_NAME,
        chains: ["Synthetic entry", "Synthetic middle", "DIRECT"],
        active: true,
      },
    ],
    requestHistory: [
      {
        ...connection,
        id: "visual-active-request",
        domain: "active-public-synthetic-request.visual-fixture.example",
        active: true,
      },
      {
        ...connection,
        id: "visual-ended-request",
        domain: "ended-public-synthetic-request.visual-fixture.example",
        active: false,
      },
    ],
    dns: {
      ...base.dns,
      statisticsAvailable: false,
      upstreams: [VISUAL_URL],
      fallbacks: ["https://fallback.visual-fixture.example/public/dns-query"],
    },
    logs: [
      {
        level: "error",
        timestamp: "2026-10-22T00:00:00Z",
        message: VISUAL_REFRESH_ERROR,
      },
      {
        level: "warn",
        timestamp: "2026-10-22T00:00:01Z",
        message: "Synthetic warning — " + VISUAL_URL,
      },
    ],
    about: {
      ...base.about,
      appVersion: "synthetic-2026.10.22-layout-preview",
      coreVersion: "synthetic-core-version-with-a-long-public-build-identifier",
      uiRuntimeVersion: "synthetic-local-SDK-layout-preview",
      privacySummary: [
        "Only public synthetic preview data. No Core, profile persistence or VPN session is initialized.",
      ],
    },
  };
}

mount(() => {
  const client = new CoreClient(),
    controls = signal(true),
    count = signal(0);
  client.preferences.set({
    language: "zh_cn",
    theme: "light",
    advanced: true,
    systemLocale: "zh-CN",
  });
  // The runner independently checks exact raw reconstruction, scalar bounds,
  // actual mounted chunks and the native Copy command against this public source.
  console.log(`VISUAL ERROR FIXTURE: ${JSON.stringify({
    source: VISUAL_REFRESH_ERROR,
    rows: errorTextRows(VISUAL_REFRESH_ERROR),
  })}`);
  const state = visualFixture();
  let staleHealth = false;
  const host = new DomainHost(client, () => state);
  host.onCommand = (request) => {
    console.log(`VISUAL COMMAND: ${request.command}`);
    if (request.command === "diagnostics.watchGet")
      host.reply(request, {
        enabled: false,
        group: state.proxyGroups[0].name,
        url: "https://visual-fixture.example/public/health",
      });
    else if (request.command === "diagnostics.healthGet")
      host.reply(request, {
        session: staleHealth ? "visual-stale-session" : state.vpnSessionId,
        engine: "meow",
        dns: "notApplicable",
        nodeTcp: "notTested",
        https: "failed",
        elapsedMs: 42,
      });
    else if (request.command === "engines.decision")
      host.reply(request, { selected: "meow", reasons: ["Synthetic visual fixture only"] });
    else if (request.command === "converter.loadDraft")
      host.reply(request, { ...converterDraftFixture(), source_sub_url: VISUAL_URL });
    else if (request.command === "logs.recordingStatus")
      host.reply(request, {
        enabled: false,
        archives: [
          {
            fileName: "public-synthetic-log-with-a-long-name-2026-10-22.log",
            bytes: 1024,
            active: false,
            updatedAt: "2026-10-22T00:00:00Z",
          },
        ],
        lastError: VISUAL_ERROR,
      });
    else if (request.command === "preferences.set") {
      client.preferences.set({ ...client.preferences(), ...request.payload });
      host.reply(request, client.preferences());
    } else
      host.result({
        id: request.id,
        ok: false,
        error: "Read-only synthetic preview: this operation is intentionally disabled",
      });
  };
  host.connect();
  return (
    <View style={{ flex: 1 }}>
      <View style={{ flex: 1 }}>
        <PawsApp client={client} embeddedGateway={false} />
      </View>
      <If when={controls()}>
        <View
          testID="visual-hide-controls"
          accessibilityRole="button"
          accessibilityLabel="Hide synthetic preview controls"
          onPress={() => controls.set(false)}
          style={{ height: 32, justifyContent: "center", backgroundColor: "#e4e4e7" }}
        >
          <Text style={{ fontSize: 11 }}>Synthetic only · no network/VPN · tap to hide</Text>
        </View>
      </If>
      <View
        testID="visual-stale-health"
        onPress={() => {
          staleHealth = true;
          count.set(count() + 1);
        }}
        style={{ position: "absolute", top: 0, left: 0, width: 1, height: 1 }}
      >
        <Text>{count()}</Text>
      </View>
    </View>
  );
});

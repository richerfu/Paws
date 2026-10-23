import { mount, signal, Text, View } from "nsh";
import { Action } from "../src/components";
import { ABOUT_LINKS } from "../src/about";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
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
        lastRefreshError: (VISUAL_ERROR + "\n").repeat(12),
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
        message: (VISUAL_ERROR + "\n").repeat(12),
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

export const ABOUT_NSH_VERSION =
  "c8779bac2365a790efd9c7aa91d3fb635815c66e+dirty.worktree-sha256-" + "0123456789abcdef".repeat(8);
export const ABOUT_CORE_VERSION =
  "e9a376c-public-local-adaptation+dirty-worktree-sha256-" + "fedcba9876543210".repeat(6);

mount(() => {
  const client = new CoreClient(),
    controls = signal(true),
    links = signal(0);
  client.preferences.set({
    language: "zh_cn",
    theme: "light",
    advanced: true,
    systemLocale: "zh-CN",
  });
  const state = visualFixture();
  state.about = {
    ...state.about,
    uiRuntimeVersion: ABOUT_NSH_VERSION,
    coreVersion: ABOUT_CORE_VERSION,
    appVersion: "2026.10.22-public-synthetic-build." + "0123456789abcdef".repeat(4),
    mihomoVersion: "v1.19.16+8597778c7df410ebe88c57e59eaab32d203156df-public-synthetic-local-build",
    singBoxVersion:
      "1.13.0-alpha.22+628cb31ffa79cffffd34c2f9cde6cae044e4fc12-public-synthetic-local-build",
    meowRsVersion: "0.22.0-public-synthetic-local-build." + "abcdef0123456789".repeat(4),
    rustVersion: "rustc 1.99.0-nightly (public-synthetic-2026-10-22-local-build)",
  };
  const host = new DomainHost(client, () => state);
  host.onCommand = (request) => {
    if (request.command === "platform.openUrl") {
      const index = ABOUT_LINKS.findIndex((link) => link.url === request.payload.url);
      if (index < 0) throw new Error("About preview rejected a non-public/unexpected URL");
      links.set(links() + 1);
      console.log(`PASS ABOUT LINK: ${index}`);
      host.reply(request, null);
    } else if (request.command === "diagnostics.watchGet")
      host.reply(request, {
        enabled: false,
        group: state.proxyGroups[0].name,
        url: "https://visual-fixture.example/public/health",
      });
    else if (request.command === "diagnostics.healthGet")
      host.reply(request, {
        session: state.vpnSessionId,
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
        error: "Read-only public preview: this operation is intentionally disabled",
      });
  };
  host.connect();
  return (
    <View style={{ flex: 1 }}>
      <View style={{ flex: 1 }}>
        <PawsApp client={client} embeddedGateway={false} />
      </View>
      <If when={controls()}>
        <View style={{ height: 72, padding: 4, gap: 4, backgroundColor: "#e4e4e7" }}>
          <Text style={{ fontSize: 11, color: "#18181b" }}>
            Public synthetic only · no network/VPN · links {links()}
          </Text>
          <View style={{ flexDirection: "row", gap: 4 }}>
            <Action
              id="about-preview-language"
              small
              secondary
              label="中文/EN"
              onPress={() =>
                client.preferences.set({
                  ...client.preferences(),
                  language: client.preferences().language === "en" ? "zh_cn" : "en",
                })
              }
            />
            <Action
              id="about-preview-theme"
              small
              secondary
              label="Light/Dark"
              onPress={() =>
                client.preferences.set({
                  ...client.preferences(),
                  theme: client.preferences().theme === "dark" ? "light" : "dark",
                })
              }
            />
            <Action
              id="about-preview-hide"
              small
              secondary
              label="Hide"
              onPress={() => controls.set(false)}
            />
          </View>
        </View>
      </If>
    </View>
  );
});

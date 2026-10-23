import { computed, mount, signal, Text } from "nsh";
import { CoreClient } from "../src/client";
import { fixture } from "./fixture";
import { DraftEdits, draftDecision } from "../src/draft";
import { isChinese } from "../src/locale";
import { connectionState } from "../src/connection-state";
import {
  matchesGeodata,
  matchesLog,
  matchesProfile,
  matchesRule,
} from "../src/query";
import { selectPanelLayout, translator, useLifetime } from "../src/components";
import { vpnIntentTrace } from "./VpnIntent";
import {
  effectiveGroupLeaf,
  primarySelectedGroupLeaf,
  groupMembers,
  proxyGroupSummary,
  groupSelectionMode,
} from "../src/proxy-groups";
import { rulePreview } from "../src/manual-rule";
import { parentPage } from "../src/app";
import { converterDraftTrace } from "./ConverterDraft";
import { domainStateTrace } from "./DomainState";
import { yamlSummaryTrace } from "./YamlSummary";

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

mount(() => {
  yamlSummaryTrace();
  const output = signal("RUNNING");
  const alive = useLifetime();
  const client = new CoreClient(),
    initial = fixture();
  const compactSelect = selectPanelLayout(
    { pageX: 16, pageY: 100, width: 358, height: 36 },
    { width: 390, height: 844 },
    { top: 32, bottom: 24, left: 0, right: 0 },
    0,
    2,
    true,
  );
  assert(
    compactSelect.height === 102,
    "short Select clipped row content inside padding and borders",
  );
  const keyboardSelect = selectPanelLayout(
    { pageX: 10, pageY: 500, width: 800, height: 36 },
    { width: 390, height: 844 },
    { top: 32, bottom: 24, left: 12, right: 12 },
    360,
    12,
    true,
  );
  assert(
    keyboardSelect.x >= 20 &&
      keyboardSelect.x + keyboardSelect.width <= 370 &&
      keyboardSelect.y >= 40 &&
      keyboardSelect.y + keyboardSelect.height <= 476,
    "Select escaped safe area or covered the keyboard",
  );
  const translate = translator(client);
  let translationRuns = 0;
  const translated = computed(() => {
    translationRuns++;
    return translate("中文", "English");
  });
  assert(translated() === "中文" && translationRuns === 1, "initial translation did not resolve");
  client.preferences.set({ ...client.preferences(), theme: "dark" });
  assert(
    translated() === "中文" && translationRuns === 1,
    "theme invalidated translated option identities",
  );
  client.preferences.set({ ...client.preferences(), advanced: true });
  assert(
    translated() === "中文" && translationRuns === 1,
    "advanced preference invalidated translated option identities",
  );
  client.preferences.set({ ...client.preferences(), language: "en" });
  assert(
    translated() === "English" && Number(translationRuns) === 2,
    "language change did not update translations",
  );
  const draftEdits = new DraftEdits(),
    saveTicket = draftEdits.ticket();
  draftEdits.edit();
  assert(!draftEdits.unchangedSince(saveTicket), "save receipt cleared an edit made while saving");
  assert(matchesProfile(initial.profiles[0], "当前"), "active profile query lost");
  assert(matchesProfile(initial.profiles[0], "backup"), "backup profile query lost");
  assert(
    matchesProfile({ ...initial.profiles[0], refreshDue: true }, "due"),
    "due profile query lost",
  );
  assert(
    matchesRule({ ...initial.rules[0], enabled: false }, "disabled"),
    "disabled rule query lost",
  );
  assert(
    matchesGeodata({ name: "geoip", path: "geoip.dat", exists: false }, "缺失"),
    "missing geodata query lost",
  );
  assert(
    matchesLog({ level: "warning", message: "Fallback", timestamp: "now" }, "WARNING", "warn"),
    "warning level or query normalization lost",
  );
  assert(
    connectionState({
      ...initial,
      vpnRunning: true,
      vpnLifecycle: "connected",
      networkProtected: false,
    }) === "unprotected",
    "running VPN falsely reported protection",
  );
  assert(
    connectionState({
      ...initial,
      vpnRunning: true,
      vpnLifecycle: "protect-failed",
      networkProtected: true,
    }) === "failed",
    "failed lifecycle falsely reported protection",
  );
  assert(
    connectionState({
      ...initial,
      vpnRunning: true,
      vpnLifecycle: "starting",
      networkProtected: true,
    }) === "connecting",
    "starting lifecycle falsely reported protection",
  );
  assert(
    connectionState({
      ...initial,
      vpnRunning: true,
      vpnLifecycle: "connected",
      networkProtected: true,
    }) === "protected",
    "authoritative connected protection lost",
  );
  const nested = [
    { ...initial.proxyGroups[0], name: "Proxy", selected: "Region" },
    { ...initial.proxyGroups[0], name: "Region" },
  ];
  assert(
    effectiveGroupLeaf(nested, "Proxy") === "Test node" &&
      primarySelectedGroupLeaf(nested) === "Test node",
    "selected subgroup did not resolve to the effective node",
  );
  assert(
    effectiveGroupLeaf(
      [
        { ...nested[0], selected: "Region" },
        { ...nested[1], selected: "Proxy" },
      ],
      "Proxy",
    ) === undefined,
    "cyclic proxy group did not terminate",
  );
  assert(
    groupMembers(
      [
        ...nested,
        {
          ...nested[0],
          name: "GLOBAL",
          proxies: [
            { name: "Proxy", proxyType: "Selector", selected: false },
            { name: "DIRECT", proxyType: "DIRECT", selected: false },
            ...initial.proxyGroups[0].proxies,
          ],
        },
      ],
      {
        ...nested[0],
        name: "GLOBAL",
        proxies: [
          { name: "Proxy", proxyType: "Selector", selected: false },
          { name: "DIRECT", proxyType: "DIRECT", selected: false },
          ...initial.proxyGroups[0].proxies,
        ],
      },
    ).length === 1,
    "GLOBAL exposed groups or built-in routes as subscription nodes",
  );
  assert(
    proxyGroupSummary([...nested, { ...nested[0], name: "GLOBAL" }]).groups === 2 &&
      proxyGroupSummary([...nested, { ...nested[0], name: "GLOBAL" }]).members === 2,
    "GLOBAL duplicated the policy group summary",
  );
  assert(
    groupSelectionMode({ ...initial.proxyGroups[0], fixed: undefined, groupType: "Selector" }) ===
      "manual" &&
      groupSelectionMode({
        ...initial.proxyGroups[0],
        fixed: undefined,
        groupType: "LoadBalance",
      }) === "policy" &&
      groupSelectionMode(initial.proxyGroups[0]) === "automatic" &&
      groupSelectionMode({ ...initial.proxyGroups[0], fixed: "Test node" }) === "pinned",
    "manual/automatic policy/pinned group semantics changed",
  );
  assert(
    rulePreview("domain-suffix", ".Example.COM.", "DIRECT") === "DOMAIN-SUFFIX,example.com,DIRECT",
    "manual domain suffix preview changed",
  );
  assert(
    rulePreview("ip-cidr", "2001:db8::1", "Proxy") === "IP-CIDR6,2001:db8::1/128,Proxy",
    "manual IP rule preview lost exact mask",
  );
  assert(
    parentPage("privacy") === "about" &&
      parentPage("proxies") === "profiles" &&
      !parentPage("dashboard"),
    "original navigation parents changed",
  );
  assert(
    draftDecision(1, 1, true) === "unchanged",
    "unchanged configuration conflicted with draft",
  );
  assert(draftDecision(1, 2, true) === "conflict", "incoming configuration overwrote dirty draft");
  assert(
    draftDecision(1, 2, false) === "reload",
    "pristine draft did not accept updated configuration",
  );
  assert(
    !isChinese({ language: "en", theme: "system", advanced: false, systemLocale: "zh-CN" }),
    "explicit English fell back to Chinese",
  );
  assert(
    isChinese({ language: "system", theme: "system", advanced: false, systemLocale: "zh-Hans-CN" }),
    "system Chinese not resolved",
  );
  client.acceptResult({ id: 0, ok: false, error: "observer failed" });
  assert(client.error() === "observer failed", "observer error was hidden");
  const start = client.send("vpn.start");
  assert(client.vpnOperation() === "start", "start was not tracked");
  const duplicate = client.send("vpn.start").then(
    () => {
      throw new Error("duplicate start accepted");
    },
    () => undefined,
  );
  const stop = client.send("vpn.stop");
  assert(client.vpnOperation() === "stop", "stop could not cancel pending start");
  client.acceptResult({ id: 2, ok: true, data: null });
  assert(client.vpnOperation() === undefined, "stop completion was ignored");
  client.acceptResult({ id: 1, ok: true, data: null });
  assert(client.vpnOperation() === undefined, "late start completion changed current intent");
  const unsupported = client.run("dns.flush", {}, "Cache flushed");
  client.acceptResult({
    id: 3,
    ok: false,
    error: "The active engine does not support DNS cache flushing",
  });
  void Promise.all([
    start,
    stop,
    duplicate,
    unsupported,
    converterDraftTrace(alive),
    vpnIntentTrace(),
    domainStateTrace(),
  ])
    .then(() => {
      assert(client.error().includes("does not support"), "unsupported engine response was hidden");
      assert(client.notice() === "", "unsupported engine response showed success");
      client.dispose();
      console.log("PASS: original control semantics, observer errors and VPN cancellation");
      output.set("PASS");
    })
    .catch((error) => {
      console.error(`Error: ${error instanceof Error ? error.message : String(error)}`);
      output.set("FAILED");
    });
  return <Text testID="state-tests">{output()}</Text>;
});

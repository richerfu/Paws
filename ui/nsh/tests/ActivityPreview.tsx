import { mount, signal, Text, View } from "nsh";
import { Action } from "../src/components";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import type { Connection } from "../src/model";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

/** Public data and real production Activity rows; no Core/network/VPN. */
export const ACTIVITY_LONG_HOST =
  "public-synthetic-unbroken-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-".repeat(3) + ".example";

mount(() => {
  const client = new CoreClient(),
    controls = signal(true);
  client.preferences.set({
    language: "zh_cn",
    theme: "light",
    advanced: true,
    systemLocale: "zh-CN",
  });
  const state = fixture(),
    base = state.connections[0];
  const rows: Connection[] = [
    { ...base, id: "short-active", domain: "a.example", active: true },
    {
      ...base,
      id: "long-ended",
      domain: ACTIVITY_LONG_HOST,
      destinationIp: "",
      active: false,
      proxy: "Public synthetic proxy route — " + ACTIVITY_LONG_HOST,
      chains: ["public entry", "public middle", "DIRECT"],
    },
    {
      ...base,
      id: "host-fallback",
      domain: "",
      host: "public-host-only.example",
      destinationIp: "",
      active: true,
    },
    {
      ...base,
      id: "ip-fallback",
      domain: "",
      host: "",
      destinationIp: "203.0.113.99",
      network: "udp",
      active: false,
    },
  ];
  state.connections = rows.map((row) => ({ ...row, active: true }));
  state.requestHistory = rows;
  const host = new DomainHost(client, () => state);
  host.onCommand = (request) =>
    host.result({
      id: request.id,
      ok: false,
      error: "Public Activity preview: writes and external operations are disabled",
    });
  host.connect();
  return (
    <View style={{ flex: 1 }}>
      <View style={{ flex: 1 }}>
        <PawsApp client={client} embeddedGateway={false} />
      </View>
      <If when={controls()}>
        <View style={{ height: 72, padding: 4, gap: 4, backgroundColor: "#e4e4e7" }}>
          <Text style={{ fontSize: 11, color: "#18181b" }}>
            Public synthetic Activity · no Core/network/VPN
          </Text>
          <View style={{ flexDirection: "row", gap: 4 }}>
            <Action
              id="activity-preview-language"
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
              id="activity-preview-theme"
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
              id="activity-preview-hide"
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

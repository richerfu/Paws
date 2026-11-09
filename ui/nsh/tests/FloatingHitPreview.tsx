import { cleanup, effect, host, List, mount, signal, Text, View } from "nsh";
import { App, type AppContext } from "../src/components";
import { CoreClient } from "../src/client";
import { FloatingVpnPowerButton } from "../src/floating-vpn";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import type { CoreRequest } from "../src/model";

/**
 * Device-only probe; VPN commands only update the synthetic DomainHost below.
 * Over the floating control, lower down/click/hover/scroll must stay unchanged.
 * Exposed rows must still click and scroll. Restore ui/nsh/App.tsx afterward.
 */
mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const top = signal(0), lowerDown = signal(0), lowerClick = signal(0), hover = signal(0), scroll = signal(0);
  const server = new DomainHost(client, () => state);
  let held: CoreRequest | undefined;
  server.onCommand = (request) => {
    if (request.command === "probe.hold") { held = request; return; }
    if (request.command !== "vpn.start" && request.command !== "vpn.stop") throw new Error(request.command);
    top.set(top() + 1);
    host.post("paws.test.report", { command: request.command });
    const running = request.command === "vpn.start";
    state = { ...state, revision: state.revision + 1, running, vpnRunning: running,
      networkProtected: running, vpnLifecycle: running ? "connected" : "stopped",
      activeEngine: running ? "meow" : null };
    server.reply(request, null, ["status"]);
  };
  client.domains.setPage(["config", "status"]);
  server.connect();
  cleanup(host.on("paws.test.hold-action", (holding: boolean) => {
    if (holding) void client.run("probe.hold");
    else if (held) { server.reply(held, null, []); held = undefined; }
  }));
  effect(() => host.post("paws.test.busy", client.pending() > 0));
  // The production floating control only consumes the client and translator.
  const ui = { client, t: (zh: string, _en: string) => zh } as AppContext;
  effect(() => console.log(`[PawsHitProbe] top=${top()} down=${lowerDown()} click=${lowerClick()} hover=${hover()} scroll=${Math.round(scroll())}`));
  return (
    <App.Provider value={ui}>
      <View style={{ flex: 1, backgroundColor: "#f4f4f2", paddingTop: "env(safe-area-inset-top)", paddingBottom: "env(safe-area-inset-bottom)" }}>
        <View style={{ height: 56, paddingHorizontal: 16, justifyContent: "center" }}>
          <Text style={{ fontSize: 16, lineHeight: 22, color: "#16171a" }}>Interaction probe — mock VPN</Text>
        </View>
        <List each={[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]} estimatedItemSize={140}
          onScroll={(event) => scroll.set(event.offset.y)}
          style={{ flex: 1 }} contentStyle={{ padding: 16 }}>
          {(row) => <View
            testID={`probe-row-${row()}`} role="button" accessibilityLabel={`Probe row ${row()}`}
            onPressDown={() => lowerDown.set(lowerDown() + 1)}
            onPress={() => lowerClick.set(lowerClick() + 1)}
            onMouseEnter={() => hover.set(hover() + 1)}
            onMouseLeave={() => hover.set(Math.max(0, hover() - 1))}
            hoverStyle={{ backgroundColor: "#c7e7ff" }} activeStyle={{ opacity: 0.55 }}
            style={{ height: 128, marginBottom: 12, backgroundColor: "#ffffff", borderRadius: 12,
              borderWidth: 1, borderColor: "#d8d8d1", justifyContent: "center", padding: 20 }}>
            <Text style={{ fontSize: 18, color: "#16171a" }}>Lower row {row()}</Text>
          </View>}
        </List>
        <View style={{ height: 64, justifyContent: "center", paddingHorizontal: 16 }}>
          <Text testID="probe-counts" style={{ fontSize: 12, lineHeight: 18, color: "#16171a" }}>
            top={top()} · lower down={lowerDown()} · lower click={lowerClick()} · hover={hover()} · scroll={Math.round(scroll())}
          </Text>
        </View>
        <If when={client.state()}><FloatingVpnPowerButton visible /></If>
      </View>
    </App.Provider>
  );
});

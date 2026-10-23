import { mount, signal, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { fixture } from "./fixture";
import { DomainHost } from "./DomainHost";

// Uses the production component tree and state reducer. Only the native
// source of states is replaced, so trace can measure actual mounted work.
mount(() => {
  const client = new CoreClient();
  const ticks = signal(0);
  let state = fixture();
  const host = new DomainHost(client, () => state);
  host.connect();
  const telemetry = () => {
    state = {
      ...state,
      revision: state.revision + 1,
      traffic: {
        ...state.traffic,
        downloadBytes: state.traffic.downloadBytes + 1024,
        downloadSpeed: state.traffic.downloadSpeed + 512,
      },
      profiles: state.profiles.map((profile) => ({
        ...profile,
        downloadBytes: profile.downloadBytes + 1024,
      })),
      logs: [
        ...state.logs,
        { level: "info", timestamp: String(state.revision), message: "Telemetry update" },
      ],
    };
    // Metadata alone invalidates active interests. The production client sends
    // explicit per-domain queries; no full state or delta crosses this boundary.
    host.change(["traffic", "summary", "profiles", "dns"]);
    ticks.set(ticks() + 1);
    console.log("PERF: telemetry state applied");
  };
  return (
    <>
      <PawsApp client={client} embeddedGateway={false} />
      <View
        testID="perf-telemetry"
        accessibilityRole="button"
        accessibilityLabel="Apply telemetry state"
        onPress={telemetry}
        style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0, opacity: 0 }}
      >
        <Text>{ticks()}</Text>
      </View>
    </>
  );
});

import { effect, mount, Text, untracked, View } from "nsh";
import { CoreClient, CoreGateway } from "../src/client";
import { fixture } from "./fixture";
import { fixtureDomain } from "./DomainHost";
import type { DomainInterest } from "../src/model";

mount(() => {
  const client = new CoreClient(),
    seen: string[] = [];
  effect(() => {
    const request = client.request();
    if (!request) return;
    // A test native host ACKs only after this prop has had a committed frame.
    // The acceptance runner verifies the real ComponentProps stream separately.
    setTimeout(
      () =>
        untracked(() => {
          if (!request.command.startsWith("state.")) seen.push(request.command);
          client.acceptReceived(request.id);
          if (request.command === "state.subscribe")
            client.acceptResult({
              id: request.id,
              ok: true,
              data: {
                generation: request.payload.generation,
                revision: 1,
                changes: (request.payload.domains as DomainInterest[]).map((interest) => ({
                  ...interest,
                  revision: 1,
                })),
              },
            });
          if (request.command === "state.query")
            client.acceptResult({
              id: request.id,
              ok: true,
              data: {
                generation: request.payload.generation,
                domain: request.payload.domain,
                revision: 1,
                configRevision: 1,
                data: fixtureDomain(fixture(), request.payload as DomainInterest),
              },
            });
          if (seen.join(",") === "vpn.start,vpn.stop,profiles.importUrl,profiles.cancelImport") {
            console.log(
              "PASS: native gateway ACK delivers synchronous start/stop and import/cancel independently of completion",
            );
          }
          // Intentionally no operation result: Stop and cancellation must still arrive.
        }),
      10,
    );
  });
  const ignore = (promise: Promise<unknown>) => {
    void promise.catch(() => undefined);
  };
  return (
    <View>
      <CoreGateway client={client} />
      <View
        testID="queue-start-stop"
        onPress={() => {
          ignore(client.send("vpn.start"));
          ignore(client.send("vpn.stop"));
        }}
      >
        <Text>Start then stop in one event</Text>
      </View>
      <View
        testID="queue-import-cancel"
        onPress={() => {
          const intentId = client.nextImportIntent();
          ignore(
            client.send("profiles.importUrl", { url: "https://fixture.example/sub", intentId }),
          );
          ignore(client.send("profiles.cancelImport", { intentId }));
        }}
      >
        <Text>Import then cancel in one event</Text>
      </View>
    </View>
  );
});

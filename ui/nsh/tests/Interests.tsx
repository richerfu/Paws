import { mount, Text, View } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";
import { testControl } from "./test-controls";

mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state);
  host.onSubscribe = (_request, change) => {
    console.log(
      `PASS INTEREST: ${change.changes
        .map((item) =>
          item.groupKey === undefined ? item.domain : `${item.domain}[${item.groupKey}]`,
        )
        .sort()
        .join(",")}`,
    );
    return false;
  };
  host.onQuery = (_request, reply) => {
    console.log(
      `PASS DOMAIN: ${reply.domain}${reply.groupKey === undefined ? "" : `[${reply.groupKey}]`}`,
    );
    return false;
  };
  host.connect();
  const tick = () => {
    const start = host.seen.length;
    state = { ...state, revision: state.revision + 1 };
    host.change([
      "summary",
      "profiles",
      "proxies",
      "rules",
      "providers",
      "geodata",
      "traffic",
      "connections",
      "requests",
      "logs",
      "dns",
      "about",
    ]);
    setTimeout(() => {
      const queried = host.seen
        .slice(start)
        .filter((request) => request.command === "state.query")
        .map((request) => request.payload.domain);
      if (
        queried.join(",") !== "summary" ||
        host.interests.some(
          (interest) => !["config", "status", "summary"].includes(interest.domain),
        )
      )
        throw new Error(
          "A retained Stack page or collapsed group kept background detail interests",
        );
      console.log(
        "PASS: production Stack forward/back releases detail interests and Home only requeries its cheap summary",
      );
    }, 10);
  };
  const metadataTick = testControl("interest-tick", tick);
  return (
    <>
      <PawsApp client={client} embeddedGateway={false} />
      <View
        testID="interest-tick"
        role="button"
        onPress={metadataTick}
        style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0, opacity: 0 }}
      >
        <Text>Metadata tick</Text>
      </View>
    </>
  );
});

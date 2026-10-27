import { testControl } from "./test-controls";
import { mount, signal, Text, View, type YNode } from "nsh";
import { PawsApp } from "../src/app";
import { CoreClient } from "../src/client";
import { errorTextRows } from "../src/page-error";
import { DomainHost } from "./DomainHost";
import { fixture } from "./fixture";

const stackLines = Array.from({ length: 58 }, (_, index) => `public synthetic stack frame ${index}`);
stackLines[0] = "Synthetic domain query failure";
stackLines[57] = "END-SYNTHETIC-STACK";
let padding = 2134 - stackLines.join("\n").length;
for (let index = 0; padding > 0; index = (index + 1) % stackLines.length, padding--)
  stackLines[index] += " ";
const STACK = stackLines.join("\n");
const LONG = "公开合成错误😀e\u0301👩‍💻".repeat(3000) + "\r\n\nEND-UNICODE-ERROR";
const QA_UNBROKEN = "公开错误👩‍💻e\u0301".repeat(512);
const testHost = globalThis as typeof globalThis & {
  __pawsPageErrorList?: YNode;
  __pawsPageErrorSetScale?: (scale: number) => void;
};

/** Real app, client, query failure, Retry and List commands. The runner only
 * captures the copied List's ref to drive its actual scrollToIndex command. */
mount(() => {
  const client = new CoreClient();
  let state = fixture();
  const host = new DomainHost(client, () => state);
  const tick = signal(0);
  let message = STACK,
    fail = false,
    failRules = false,
    failures = 0,
    selections = 0,
    scanIndex = 0,
    layoutScale = 1;
  const assert = (condition: unknown, reason: string) => {
    if (!condition) throw new Error(reason);
  };
  assert(new TextEncoder().encode(STACK).length === 2134, "Stack fixture is not 2134 bytes");
  assert(new TextEncoder().encode(QA_UNBROKEN).length === 13312 &&
    Array.from(QA_UNBROKEN).length === 4608, "Public unbroken fixture changed");
  assert(errorTextRows("Public QA readable error sentence length 44!!").length === 1,
    "A normal 44-character error sentence was artificially split");
  const goldens = ["", "\r\n", "a\r\nb\rc\n", "x".repeat(160) + "\r\nlast",
    "😀".repeat(161), "e\u0301👩‍💻".repeat(120), "a\u2028b\u2029c\u0085d\u000be\u000c", STACK, LONG, QA_UNBROKEN];
  for (const source of goldens) for (const scale of [1, 2, 3, 0, -1, Infinity, NaN]) {
    const rows = errorTextRows(source, scale);
    assert(rows.map((row) => row.raw).join("") === source, "Error rows changed the full source");
    for (const row of rows) {
      const bound = Math.max(1, Math.floor(48 / (Number.isFinite(scale) ? Math.max(1, scale) : 1)));
      assert(Array.from(row.text).length <= bound, "A display chunk exceeded its scalar bound");
      assert(!/[\r\n\v\f\u0085\u2028\u2029]/.test(row.text), "A chunk retained an unbounded hard break");
      assert(!/^[\uDC00-\uDFFF]|[\uD800-\uDBFF]$/.test(row.text), "A valid surrogate pair was split");
    }
  }
  console.log("PASS PAGE ERROR: exact raw reconstruction and Unicode/hard-break bounds");
  host.onQuery = (request, reply) => {
    if (!((reply.domain === "proxies" && fail) || (reply.domain === "rules" && failRules))) return false;
    failures++;
    host.result({ id: request.id, ok: false, error: message });
    console.log(`PAGE ERROR DATA: ${JSON.stringify({ bytes: new TextEncoder().encode(message).length,
      utf16: message.length, rows: errorTextRows(message).length, failures })}`);
    return true;
  };
  host.onCommand = (request) => {
    assert(request.command === "proxies.select", `Unexpected error fixture command: ${request.command}`);
    selections++;
    assert(request.payload.group === "Proxy" && request.payload.name === "Test node", "Unexpected selected node");
    fail = true;
    state = { ...state, revision: state.revision + 1, configRevision: state.configRevision + 1 };
    host.reply(request, true, ["config", "status", "proxies"]);
  };
  host.connect();
  const port = (id: string, action: () => void) => {
    const press = testControl(`error-${id}`, () => { action(); tick.set(tick() + 1); });
    return <View testID={`error-${id}`} onPress={press}><Text>{id}</Text></View>;
  };
  return (
    <View style={{ flex: 1 }}>
      <PawsApp client={client} embeddedGateway={false} />
      <View style={{ position: "absolute", width: 1, height: 1, top: 0, left: 0 }}>
        <Text>{tick()}</Text>
        {port("long", () => { message = LONG; })}
        {port("qa-unbroken", () => { message = "\n".repeat(24) + QA_UNBROKEN; })}
        {[1, 2, 3].map((scale) => port(`scale-${scale}`, () => {
          assert(testHost.__pawsPageErrorSetScale, "Owned scaled-layout port was not registered");
          layoutScale = scale;
          scanIndex = 0;
          testHost.__pawsPageErrorSetScale!(scale);
        }))}
        {port("initial-failure", () => { fail = true; })}
        {port("rules-failure", () => { failRules = true; })}
        {port("allow", () => { fail = false; })}
        {port("assert-selected-failure", () => {
          assert(selections === 1 && failures === 1 && client.domains.error(["proxies"]) === message,
            "Actual selected-node query failure did not reach the domain");
          console.log("PASS PAGE ERROR: real select receipt followed by failed proxies query");
        })}
        {port("tail", () => {
          assert(testHost.__pawsPageErrorList, "Copied List ref was not registered");
          testHost.__pawsPageErrorList!.scrollToIndex({ index: errorTextRows(message, layoutScale).length - 1,
            animated: false, viewPosition: 1 });
        })}
        {port("scan", () => {
          assert(testHost.__pawsPageErrorList, "Copied List ref was not registered");
          const count = errorTextRows(message, layoutScale).length;
          const index = Math.min(scanIndex, count - 1);
          testHost.__pawsPageErrorList!.scrollToIndex({ index, animated: false, viewPosition: 0 });
          console.log(`PAGE ERROR SCAN: ${JSON.stringify({ index, count, scale: layoutScale })}`);
          scanIndex += Math.max(1, Math.floor(12 / layoutScale));
        })}
        {port("assert-recovered", () => {
          assert(client.domains.fresh(["proxies"]) && client.domains.error(["proxies"]) === "" &&
            selections <= 1, "Retry failed or replayed the selected-node mutation");
          console.log("PASS PAGE ERROR: readonly Retry recovered without replaying select");
        })}
        {port("assert-nested-closed", () => {
          assert(selections === 0 && failures === 1, "Closing details retried a query or replayed a mutation");
          console.log("PASS PAGE ERROR: nested details close does not retry or mutate");
        })}
      </View>
    </View>
  );
});

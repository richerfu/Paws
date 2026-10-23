import { CoreClient } from "../src/client";
const wait = () => new Promise<void>((resolve) => setTimeout(resolve, 12));
function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

/** Exercises the production client with actual native receipt ids and its deadline. */
export async function vpnIntentTrace() {
  const client = new CoreClient(5);
  const first = client.send("vpn.start").then(
    () => false,
    () => true,
  );
  await wait();
  assert(await first, "missing deadline rejection");
  assert(
    client.vpnUnconfirmed()?.requestId === 1 && client.vpnUnconfirmed()?.action === "start",
    "deadline lost original VPN receipt identity",
  );
  assert(client.pending() === 0, "unconfirmed operation kept unrelated controls pending");
  assert(
    await client.send("vpn.restart").then(
      () => false,
      () => true,
    ),
    "unconfirmed operation allowed a duplicate restart",
  );
  client.acceptResult({ id: 1, ok: true, data: null });
  assert(!client.vpnUnconfirmed(), "late original receipt did not resynchronize");
  const next = client.send("vpn.start").catch(() => undefined);
  await wait();
  await next;
  const unconfirmed = client.vpnUnconfirmed();
  assert(unconfirmed?.requestId === 2, "next operation identity was not retained");
  const stop = client.send("vpn.stop");
  assert(client.vpnOperation() === "stop", "explicit stop could not recover an unconfirmed start");
  client.acceptResult({ id: 2, ok: true, data: null });
  assert(client.vpnOperation() === "stop", "late start receipt cleared the newer stop intent");
  client.acceptResult({ id: 3, ok: true, data: null });
  await stop;
  assert(
    !client.vpnUnconfirmed() && !client.vpnOperation(),
    "confirmed stop did not resynchronize",
  );
  assert(
    client.nextImportIntent() === 1 && client.nextImportIntent() === 2,
    "import token was not monotonic per client",
  );
  const another = client.send("vpn.start").catch(() => undefined);
  await wait();
  await another;
  const confirm = client.send("vpn.confirm", { requestId: 4 });
  client.acceptResult({
    id: 5,
    ok: true,
    data: { requestId: 4, status: "pending", result: false, error: "" },
  });
  await confirm;
  assert(client.vpnUnconfirmed()?.requestId === 4, "pending receipt falsely confirmed VPN");
  const unavailable = client.send("vpn.confirm", { requestId: 4 });
  client.acceptResult({
    id: 6,
    ok: true,
    data: { requestId: 4, status: "unavailable", result: false, error: "receipt unavailable" },
  });
  await unavailable;
  assert(
    client.vpnUnconfirmed()?.requestId === 4 && client.error() === "receipt unavailable",
    "missing receipt was treated as success",
  );
  const failedRecovery = client.send("vpn.recover", { requestId: 4 });
  client.acceptResult({
    id: 7,
    ok: true,
    data: { requestId: 4, status: "failed", result: false, error: "OS stop failed" },
  });
  await failedRecovery;
  assert(
    client.vpnUnconfirmed()?.requestId === 4 && client.error() === "OS stop failed",
    "failed recovery cleared unknown operation",
  );
  const confirmed = client.send("vpn.confirm", { requestId: 4 });
  client.acceptResult({
    id: 8,
    ok: true,
    data: { requestId: 4, status: "succeeded", result: true, error: "" },
  });
  await confirmed;
  assert(!client.vpnUnconfirmed(), "terminal receipt lookup did not settle original operation");
  const followup = new CoreClient(5);
  const save = followup
    .send("settings.vpn", { profileId: "profile", expectedRevision: 1 })
    .catch(() => undefined);
  await wait();
  await save;
  assert(!followup.vpnUnconfirmed(), "UI guessed a platform effect before the host reported one");
  followup.acceptOperation({ requestId: 1, action: "restart" });
  assert(
    followup.vpnUnconfirmed()?.requestId === 1,
    "late platform effect lost an already-expired durable request",
  );
  const confirmSave = followup.send("vpn.confirm", { requestId: 1 });
  followup.acceptResult({
    id: 2,
    ok: true,
    data: { requestId: 1, status: "succeeded", result: true, error: "" },
  });
  await confirmSave;
  assert(!followup.vpnUnconfirmed(), "confirmed save followup stayed unconfirmed");
  const secondSave = followup
    .send("profiles.refresh", { profileId: "profile" })
    .catch(() => undefined);
  followup.acceptOperation({ requestId: 3, action: "restart" });
  const newerStop = followup.send("vpn.stop");
  followup.acceptOperation({ requestId: 3, action: "restart" });
  followup.acceptResult({ id: 3, ok: true, data: null });
  assert(followup.vpnOperation() === "stop", "late save followup overwrote the newer Stop intent");
  followup.acceptResult({ id: 4, ok: true, data: null });
  await Promise.all([secondSave, newerStop]);
  followup.dispose();
  console.log("PASS: late durable VPN effect becomes unconfirmed and cannot replace a newer Stop");
  client.dispose();
  console.log(
    "PASS: VPN deadline retains receipt, blocks repeat start and accepts late receipt or explicit stop",
  );
}

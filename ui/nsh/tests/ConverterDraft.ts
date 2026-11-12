import { CoreClient } from "../src/client";
import { ConverterDraftState, type ConverterDraft } from "../src/converter-draft";
import { converterDraftFixture } from "./fixture";

const assert = (value: unknown, message: string) => {
  if (!value) throw new Error(message);
};

/** Drive the production draft owner through delayed native command receipts. */
export async function converterDraftTrace(alive: () => boolean): Promise<void> {
  const client = new CoreClient();
  const state = new ConverterDraftState(client, {
    alive,
    saved: () => "Saved",
    changed: () => "Edits preserved",
  });
  const request = async (command: string) => {
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const sent = client.request()!;
    assert(sent.command === command, `Expected ${command}, got ${sent.command}`);
    return sent;
  };

  const load = state.load();
  let sent = await request("converter.loadDraft");
  client.acceptResult({ id: sent.id, ok: true, data: converterDraftFixture() });
  await load;
  assert(state.initialized() && !state.dirty(), "Converter draft failed to load");

  state.change({ filename: "submitted.yaml" });
  const save = state.save();
  sent = await request("converter.saveDraft");
  assert(
    (sent.payload.draft as ConverterDraft).filename === "submitted.yaml",
    "Save submitted the wrong draft",
  );
  state.change({ filename: "edited-while-saving.yaml" });
  client.acceptResult({ id: sent.id, ok: true, data: null });
  await save;
  assert(
    state.dirty() && state.draft()?.filename === "edited-while-saving.yaml",
    "Late save receipt cleared newer edits",
  );

  const saveCurrent = state.save();
  sent = await request("converter.saveDraft");
  assert(
    (sent.payload.draft as ConverterDraft).filename === "edited-while-saving.yaml",
    "Newer edits did not reach the next save",
  );
  client.acceptResult({ id: sent.id, ok: true, data: null });
  await saveCurrent;
  assert(!state.dirty(), "Unchanged successful save remained dirty");

  const parse = state.parse("https://short.example/delayed");
  sent = await request("converter.parseUrl");
  assert(
    sent.payload.input === "https://short.example/delayed",
    "Parse input did not reach the bridge",
  );
  state.change({ backend: "https://user-edit.example/sub?", filename: "keep.yaml" });
  const parsed = { ...converterDraftFixture(), filename: "parsed.yaml" };
  client.acceptResult({ id: sent.id, ok: true, data: parsed });
  await parse;
  assert(
    state.draft()?.filename === "keep.yaml" &&
      state.draft()?.backend === "https://user-edit.example/sub?",
    "Late parse overwrote newer edits",
  );
  assert(
    state.dirty() && client.error() === "Edits preserved",
    "Late parse silently discarded its conflict",
  );

  const parseCurrent = state.parse("https://short.example/current");
  sent = await request("converter.parseUrl");
  client.acceptResult({ id: sent.id, ok: true, data: parsed });
  await parseCurrent;
  assert(
    state.draft()?.filename === "parsed.yaml" && state.dirty(),
    "Unchanged parse did not fill the editable draft",
  );

  const failedSave = state.save();
  sent = await request("converter.saveDraft");
  client.acceptResult({ id: sent.id, ok: false, error: "Disk full" });
  await failedSave;
  assert(
    state.dirty() && client.error() === "Disk full" && client.notice() === "",
    "Failed save lost edits or showed success",
  );
  client.dispose();
  console.log("PASS: converter delayed save, parse conflict, retry and failure preserve edits");
}

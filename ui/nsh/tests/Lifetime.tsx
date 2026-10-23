import { mount, signal, Text } from "nsh";
import { useLifetime } from "../src/components";
import { CoreClient } from "../src/client";
import { ConverterDraftState } from "../src/converter-draft";
import { converterDraftFixture } from "./fixture";

mount(() => {
  const shown = signal(true),
    result = signal("RUNNING");
  const client = new CoreClient();
  let draft: ConverterDraftState | undefined;
  let writes = 0,
    disposed = false;
  function PendingPage() {
    const alive = useLifetime(),
      field = signal("");
    draft = new ConverterDraftState(client, {
      alive,
      saved: () => "Saved",
      changed: () => "Edits preserved",
    });
    void draft.parse("https://short.example/disposed");
    setTimeout(() => {
      disposed = !alive();
      if (alive()) {
        field.set("late result");
        writes++;
      }
      client.acceptResult({ id: 1, ok: true, data: converterDraftFixture() });
    }, 20);
    return <Text>{field()}</Text>;
  }
  setTimeout(() => shown.set(false), 0);
  setTimeout(() => {
    if (!disposed || writes !== 0) throw new Error("Disposed page accepted an asynchronous result");
    if (draft?.draft() !== undefined || draft?.dirty())
      throw new Error("Disposed converter accepted a late parse receipt");
    client.dispose();
    result.set("PASS");
    console.log("PASS: disposed page drops asynchronous results");
    console.log("PASS: disposed converter drops delayed native parse receipt");
  }, 40);
  return (
    <>
      <If when={shown()}>
        <PendingPage />
      </If>
      <Text testID="lifetime-tests">{result()}</Text>
    </>
  );
});

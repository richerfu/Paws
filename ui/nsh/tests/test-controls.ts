import { cleanup, host } from "nsh";

/** Fixture orchestration travels over the public host channel so a test can
 * release a held server reply while a real modal owns all user input. */
export function testControl(id: string, action: () => void | Promise<void>): () => void {
  host.post("paws.test.register", id);
  cleanup(host.on("paws.test.action", (payload: { id: string; token: number }) => {
    if (payload.id !== id) return;
    void Promise.resolve().then(action).then(
      () => host.post("paws.test.done", { token: payload.token }),
      (error: Error) => host.post("paws.test.done", { token: payload.token, error: error.message }),
    );
  }));
  return () => { void action(); };
}

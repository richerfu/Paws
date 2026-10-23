/** Telemetry does not change configRevision. Only a new persisted revision can
 * reload a pristine draft or conflict with edits the user has made. */
export function draftDecision(
  baselineRevision: number,
  incomingRevision: number,
  dirty: boolean,
): "unchanged" | "reload" | "conflict" {
  if (incomingRevision === baselineRevision) return "unchanged";
  return dirty ? "conflict" : "reload";
}

/** A save receipt may clear only the edits that were actually submitted. */
export class DraftEdits {
  private generation = 0;
  edit(): void {
    this.generation++;
  }
  ticket(): number {
    return this.generation;
  }
  unchangedSince(ticket: number): boolean {
    return this.generation === ticket;
  }
}

import { signal } from "nsh";
import type { CoreClient } from "./client";
import { DraftEdits } from "./draft";

export type ConverterDraft = {
  advanced: boolean;
  source_sub_url: string;
  client_type: string;
  backend: string;
  remote_config: string;
  exclude_remarks: string;
  include_remarks: string;
  filename: string;
  emoji: boolean;
  node_list: boolean;
  sort: boolean;
  udp: boolean;
  need_udp: boolean;
  tfo: boolean;
  scv: boolean;
  fdn: boolean;
  expand: boolean;
  append_type: boolean;
  insert: boolean;
  new_name: boolean;
  surge_doh: boolean;
  clash_doh: boolean;
  custom_params: { name: string; value: string }[];
  short_url_api: string;
  config_upload_api: string;
};

type DraftOwner = { alive: () => boolean; saved: () => string; changed: () => string };

/** A page owns its edits; asynchronous receipts apply only to the submitted generation. */
export class ConverterDraftState {
  readonly draft = signal<ConverterDraft | undefined>();
  readonly dirty = signal(false);
  readonly loading = signal(false);
  readonly initialized = signal(false);
  private readonly edits = new DraftEdits();

  constructor(
    private readonly client: CoreClient,
    private readonly owner: DraftOwner,
  ) {}

  change(patch: Partial<ConverterDraft>): void {
    const current = this.draft();
    if (!current) return;
    this.edits.edit();
    this.draft.set({ ...current, ...patch });
    this.dirty.set(true);
  }

  async load(): Promise<void> {
    if (this.loading()) return;
    const ticket = this.edits.ticket();
    this.loading.set(true);
    const value = await this.client.run<ConverterDraft>("converter.loadDraft");
    if (!this.owner.alive()) return;
    this.loading.set(false);
    if (value && this.edits.unchangedSince(ticket)) {
      this.draft.set(value);
      this.initialized.set(true);
      this.dirty.set(false);
    }
  }

  async parse(input: string): Promise<void> {
    const ticket = this.edits.ticket();
    const value = await this.client.run<ConverterDraft>("converter.parseUrl", { input });
    if (!this.owner.alive() || !value) return;
    if (!this.edits.unchangedSince(ticket)) {
      this.client.error.set(this.owner.changed());
      return;
    }
    this.edits.edit();
    this.draft.set(value);
    this.initialized.set(true);
    this.dirty.set(true);
  }

  async save(): Promise<void> {
    const draft = this.draft();
    if (!draft) return;
    const ticket = this.edits.ticket();
    const result = await this.client.run("converter.saveDraft", { draft }, this.owner.saved());
    if (this.owner.alive() && result !== undefined && this.edits.unchangedSince(ticket))
      this.dirty.set(false);
  }
}

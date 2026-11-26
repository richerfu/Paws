import { batch, signal, type Signal } from "nsh";
import type {
  DomainData,
  DomainInput,
  DomainInterest,
  DomainName,
  DomainQueryResult,
  DomainRevision,
  Profile,
  StateBarrier,
  StateChanged,
} from "./model";

type Phase = "idle" | "loading" | "ready" | "error";
type Entry = {
  interest: DomainInterest;
  data: Signal<DomainData[DomainName] | undefined>;
  phase: Signal<Phase>;
  error: Signal<string>;
  version: Signal<number>;
  appliedGeneration: Signal<number>;
  observation: Signal<DomainQueryResult["observation"]>;
  target: number;
  freshTarget: number;
  flight?: Promise<void>;
  failedGeneration?: number;
};
type Barrier = {
  required: DomainRevision[];
  resolve: () => void;
  reject: (error: Error) => void;
  timer: number;
};
const critical: DomainInterest[] = [{ domain: "config" }, { domain: "status" }];
export const domainKey = (interest: DomainInterest): string =>
  interest.groupKey === undefined
    ? interest.domain
    : `${interest.domain}:${JSON.stringify(interest.groupKey)}`;
const normalize = (input: DomainInput): DomainInterest =>
  typeof input === "string" ? { domain: input } : input;
const revision = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const fields: Record<DomainName, string[]> = {
  config: ["activeProfile", "mode", "vpnOptions", "controllerAccess", "networkPorts"],
  status: [
    "vpnSessionId",
    "vpnLifecycle",
    "engineLoaded",
    "activeEngine",
    "running",
    "vpnRunning",
    "networkProtected",
    "networkProtectError",
    "controllerRunning",
    "controllerAddr",
    "exitLocation",
  ],
  summary: [
    "appVersion",
    "profileCount",
    "activeProfileName",
    "globalNodeCount",
    "nodeCount",
    "groupCount",
    "primarySelectedNode",
    "globalSelectedNode",
    "latestRuleNode",
    "groups",
  ],
  profiles: ["profiles"],
  proxies: ["proxyGroups"],
  rules: ["rules"],
  providers: ["providers"],
  geodata: ["geodata"],
  traffic: [
    "traffic",
    "trafficHistory",
    "controllerDiagnostics",
    "activeProfileUsage",
    "connectionSummary",
  ],
  connections: ["connections"],
  requests: ["requestHistory"],
  logs: ["logs"],
  dns: ["dns"],
  about: ["about"],
};
const arrays = new Set([
  "profiles",
  "proxyGroups",
  "rules",
  "providers",
  "geodata",
  "trafficHistory",
  "connections",
  "requestHistory",
  "logs",
  "groups",
]);
const nullables = new Set([
  "activeProfile",
  "vpnSessionId",
  "activeEngine",
  "networkProtectError",
  "controllerAddr",
  "activeProfileName",
  "primarySelectedNode",
  "globalSelectedNode",
  "latestRuleNode",
]);
const booleans = new Set([
  "engineLoaded",
  "running",
  "vpnRunning",
  "networkProtected",
  "controllerRunning",
]);
const counts = new Set(["profileCount", "globalNodeCount", "nodeCount", "groupCount"]);
const optionalNullableNumbers: Partial<Record<DomainName, string[]>> = {
  status: ["vpnConnectedAt"],
  summary: ["primarySelectedDelayMs", "globalSelectedDelayMs"],
};
const optionalNullableStrings: Partial<Record<DomainName, string[]>> = {
  summary: ["primarySelectedGroup"],
};

function deepEqual(previous: unknown, next: unknown): boolean {
  if (Object.is(previous, next)) return true;
  if (
    !previous ||
    !next ||
    typeof previous !== "object" ||
    typeof next !== "object"
  )
    return false;
  if (Array.isArray(previous) || Array.isArray(next)) {
    if (!Array.isArray(previous) || !Array.isArray(next)) return false;
    if (previous.length !== next.length) return false;
    for (let index = 0; index < previous.length; index++) {
      if (!deepEqual(previous[index], next[index])) return false;
    }
    return true;
  }
  const prevObj = previous as Record<string, unknown>;
  const nextObj = next as Record<string, unknown>;
  const prevKeys = Object.keys(prevObj);
  const nextKeys = Object.keys(nextObj);
  if (prevKeys.length !== nextKeys.length) return false;
  for (const key of prevKeys) {
    if (!Object.prototype.hasOwnProperty.call(nextObj, key)) return false;
    if (!deepEqual(prevObj[key], nextObj[key])) return false;
  }
  return true;
}

function validateData(domain: DomainName, data: unknown): void {
  if (!data || typeof data !== "object" || Array.isArray(data))
    throw new Error(`Invalid ${domain} query data`);
  const object = data as Record<string, unknown>;
  for (const key of fields[domain]) {
    const value = object[key];
    const present = Object.prototype.hasOwnProperty.call(object, key) && value !== undefined;
    const valid = arrays.has(key)
      ? Array.isArray(value)
      : nullables.has(key)
        ? value === null || typeof value === "string"
        : booleans.has(key)
          ? typeof value === "boolean"
          : counts.has(key)
            ? revision(value)
            : key === "mode"
              ? ["rule", "global", "direct"].includes(String(value))
              : key === "vpnLifecycle"
                ? [
                    "stopped",
                    "engine-loaded",
                    "starting",
                    "connected",
                    "protect-failed",
                    "failed",
                  ].includes(String(value))
                : key === "appVersion"
                  ? typeof value === "string"
                  : key === "activeProfileUsage"
                    ? value === null || (typeof value === "object" && !Array.isArray(value))
                    : !!value && typeof value === "object" && !Array.isArray(value);
    if (!present || !valid)
      throw new Error(`Invalid ${domain} query: ${present ? "invalid" : "missing"} ${key}`);
  }
  for (const key of optionalNullableNumbers[domain] || []) {
    if (!Object.prototype.hasOwnProperty.call(object, key)) continue;
    const value = object[key];
    if (
      value !== undefined &&
      value !== null &&
      (typeof value !== "number" || !Number.isFinite(value) || value < 0)
    )
      throw new Error(`Invalid ${domain} query: invalid ${key}`);
  }
  for (const key of optionalNullableStrings[domain] || []) {
    if (!Object.prototype.hasOwnProperty.call(object, key)) continue;
    const value = object[key];
    if (value !== undefined && value !== null && typeof value !== "string")
      throw new Error(`Invalid ${domain} query: invalid ${key}`);
  }
}

/** A receipt can be committed even when its subsequent view query fails. */
export class CommittedStateError extends Error {
  constructor(reason: string) {
    super(
      `Changes were applied, but state synchronization failed: ${reason}. Reload the state before continuing.`,
    );
  }
}

export type InitialDomainProjection<D extends DomainName = DomainName> = Omit<
  DomainQueryResult<D>,
  "generation"
>;

/** One active interest set; events invalidate keys and never carry domain data. */
export class DomainStore {
  readonly configRevision = signal(0);
  readonly latestRevision = signal(0);
  readonly subscriptionError = signal("");
  private readonly entries = new Map<string, Entry>();
  private readonly leases = new Map<number, DomainInterest[]>();
  private readonly barriers = new Set<Barrier>();
  private readonly profileUsage = signal(
    new Map<string, { uploadBytes: number; downloadBytes: number }>(),
  );
  private profilesConfigRevision = -1;
  private page: DomainInterest[] = [];
  private interests = new Map<string, DomainInterest>();
  private generation = 0;
  private subscribedGeneration = -1;
  private subscribingGeneration = -1;
  private leaseSequence = 0;
  private connected = false;
  private disposed = false;
  private scheduled = false;

  constructor(
    private readonly send: <T>(command: string, payload: Record<string, unknown>) => Promise<T>,
    private readonly reportError: (message: string) => void,
    private readonly timeoutMs = 15_000,
  ) {}
  get observing(): boolean {
    return this.connected;
  }

  connect(): void {
    if (!this.connected && !this.disposed) {
      this.connected = true;
      this.schedule();
    }
  }
  /** Seeds cold-start projections from host launch params before Frame 1 mounts.
   * Leaves version and appliedGeneration unset so live gateway subscribe/query
   * still reconciles and fences imperative mutations until confirmed. */
  seed(projections: readonly InitialDomainProjection[]): void {
    if (this.disposed || !Array.isArray(projections) || !projections.length) return;
    batch(() => {
      for (const value of projections) {
        if (
          !value ||
          typeof value !== "object" ||
          !(value.domain in fields) ||
          (value.groupKey !== undefined && typeof value.groupKey !== "string") ||
          !revision(value.revision) ||
          !revision(value.configRevision)
        )
          continue;
        try {
          validateData(value.domain, value.data);
        } catch {
          continue;
        }
        const entry = this.entry({
          domain: value.domain,
          ...(value.groupKey !== undefined ? { groupKey: value.groupKey } : {}),
        });
        if (entry.data() || entry.version() >= 0) continue;
        this.apply(entry, { ...value, generation: 0 });
        if (value.observation !== undefined) entry.observation.set(value.observation);
        entry.phase.set("ready");
        this.latestRevision.set(Math.max(this.latestRevision(), value.revision));
        if (value.domain === "config") {
          this.configRevision.set(Math.max(this.configRevision(), value.configRevision));
        }
      }
    });
  }
  setPage(inputs: DomainInput[]): void {
    this.page = inputs.map(normalize);
    this.schedule();
  }
  acquire(inputs: DomainInput[]): () => void {
    const id = ++this.leaseSequence;
    this.leases.set(id, inputs.map(normalize));
    this.schedule();
    return () => {
      if (this.leases.delete(id)) this.schedule();
    };
  }
  data<D extends DomainName>(domain: D, groupKey?: string): DomainData[D] | undefined {
    return this.entry({ domain, groupKey }).data() as DomainData[D] | undefined;
  }
  has(inputs: DomainInput[]): boolean {
    return inputs.every((input) => !!this.entry(normalize(input)).data());
  }
  fresh(inputs: DomainInput[]): boolean {
    return inputs.every((input) => {
      const entry = this.entry(normalize(input));
      return (
        entry.phase() === "ready" &&
        entry.appliedGeneration() === this.generation &&
        entry.version() >= entry.freshTarget
      );
    });
  }
  readyForCommand(inputs: DomainInput[]): boolean {
    return inputs.every((input) => {
      const interest = normalize(input);
      if (interest.domain !== "profiles") return this.fresh([interest]);
      const entry = this.entry(interest);
      // Profiles also refresh their traffic counters every second. Those reads
      // do not invalidate edits based on the same loaded configuration. Keep
      // generation, query failures and real config changes as write fences.
      return (
        this.fresh(["config"]) &&
        !!entry.data() &&
        entry.phase() !== "error" &&
        entry.appliedGeneration() === this.generation &&
        this.profilesConfigRevision === this.configRevision()
      );
    });
  }
  error(inputs: DomainInput[]): string {
    return (
      this.subscriptionError() ||
      inputs.map((input) => this.entry(normalize(input)).error()).find(Boolean) ||
      ""
    );
  }
  issues(inputs: DomainInput[]): string {
    return [
      ...new Set(
        inputs.flatMap(
          (input) =>
            this.entry(normalize(input))
              .observation()
              ?.issues.map((issue) => `${issue.field}: ${issue.reason}`) || [],
        ),
      ),
    ].join("\n");
  }
  retry(inputs?: DomainInput[]): void {
    if (this.subscriptionError()) {
      this.subscribedGeneration = -1;
      this.schedule(true);
      return;
    }
    const toQuery: Entry[] = [];
    batch(() => {
      for (const input of inputs || [...this.interests.values()]) {
        const entry = this.entry(normalize(input));
        entry.failedGeneration = undefined;
        entry.appliedGeneration.set(-1);
        entry.error.set("");
        if (this.interests.has(domainKey(entry.interest))) {
          entry.phase.set("loading");
          toQuery.push(entry);
        }
      }
    });
    for (const entry of toQuery) this.query(entry);
  }

  changed(value: StateChanged): void {
    this.applyChanged(value);
  }

  private applyChanged(
    value: StateChanged,
    silentContinuous?: ReadonlySet<string>,
  ): void {
    if (
      this.disposed ||
      value.generation !== this.generation ||
      !revision(value.revision) ||
      !Array.isArray(value.changes)
    )
      return;
    const toQuery: Entry[] = [];
    batch(() => {
      for (const change of value.changes) {
        if (
          !change ||
          !(change.domain in fields) ||
          !revision(change.revision) ||
          (change.groupKey !== undefined && typeof change.groupKey !== "string")
        )
          continue;
        const entry = this.entry(change),
          key = domainKey(change);
        if (!this.interests.has(key)) continue;
        entry.target = Math.max(entry.target, change.revision);
        if (silentContinuous?.has(key) && entry.data() && entry.phase() === "ready") {
          // Continuously subscribed domain whose subscribe receipt carries a newer
          // global revision (e.g., from background telemetry ticks): silently catch
          // up version in the background without flashing controls to loading.
          entry.failedGeneration = undefined;
          entry.appliedGeneration.set(this.generation);
          if (entry.version() < entry.target) toQuery.push(entry);
          continue;
        }
        entry.freshTarget = Math.max(entry.freshTarget, change.revision);
        if (
          entry.data() &&
          entry.phase() === "ready" &&
          entry.version() >= entry.target
        ) {
          entry.failedGeneration = undefined;
          entry.appliedGeneration.set(this.generation);
        } else if (
          entry.appliedGeneration() !== this.generation ||
          entry.version() < entry.target
        ) {
          entry.failedGeneration = undefined;
          entry.phase.set("loading");
          toQuery.push(entry);
        }
      }
    });
    for (const entry of toQuery) this.query(entry);
    this.checkBarriers();
  }

  waitFor(barrier: StateBarrier): Promise<void> {
    if (this.disposed) return Promise.reject(new CommittedStateError("View disposed"));
    if (!Array.isArray(barrier.required))
      return Promise.reject(new CommittedStateError("Invalid synchronization receipt"));
    return new Promise<void>((resolve, reject) => {
      const pending: Barrier = { required: barrier.required, resolve, reject, timer: 0 };
      pending.timer = setTimeout(() => {
        this.barriers.delete(pending);
        reject(new CommittedStateError("The requested state did not arrive in time"));
      }, this.timeoutMs);
      this.barriers.add(pending);
      const toQuery: Entry[] = [];
      let invalid = false;
      batch(() => {
        for (const required of pending.required) {
          if (!required || !(required.domain in fields) || !revision(required.revision)) {
            invalid = true;
            return;
          }
          if (!this.interests.has(domainKey(required))) continue;
          const entry = this.entry(required);
          entry.target = Math.max(entry.target, required.revision);
          entry.freshTarget = Math.max(entry.freshTarget, required.revision);
          if (!this.fresh([entry.interest]) && entry.failedGeneration !== this.generation) {
            entry.phase.set("loading");
            toQuery.push(entry);
          }
        }
      });
      if (invalid) {
        this.finishBarrier(pending, new CommittedStateError("Invalid domain revision"));
        return;
      }
      for (const entry of toQuery) this.query(entry);
      this.checkBarriers();
    });
  }

  private entry(interest: DomainInterest): Entry {
    const key = domainKey(interest);
    let value = this.entries.get(key);
    if (!value) {
      value = {
        interest: {
          domain: interest.domain,
          ...(interest.groupKey !== undefined ? { groupKey: interest.groupKey } : {}),
        },
        data: signal<DomainData[DomainName]>(),
        phase: signal<Phase>("idle"),
        error: signal(""),
        observation: signal<DomainQueryResult["observation"]>(),
        version: signal(-1),
        appliedGeneration: signal(-1),
        target: 0,
        freshTarget: 0,
      };
      this.entries.set(key, value);
    }
    return value;
  }

  private schedule(force = false): void {
    if (force) {
      this.subscribedGeneration = -1;
      this.subscribingGeneration = -1;
    }
    if (this.scheduled || this.disposed) return;
    this.scheduled = true;
    void Promise.resolve().then(() => {
      this.scheduled = false;
      this.reconcile();
    });
  }
  private reconcile(): void {
    if (this.disposed) return;
    const prevInterests = this.interests;
    const prevGeneration = this.generation;
    const desired = new Map<string, DomainInterest>();
    for (const interest of [...critical, ...this.page, ...[...this.leases.values()].flat()])
      desired.set(domainKey(interest), interest);
    const same =
      desired.size === prevInterests.size &&
      [...desired.keys()].every((key) => prevInterests.has(key));
    this.interests = desired;
    this.checkBarriers(); // A departed page cannot hold a committed command indefinitely.
    if (
      !this.connected ||
      (same &&
        (this.subscribedGeneration === this.generation ||
          this.subscribingGeneration === this.generation))
    )
      return;
    const generation = ++this.generation;
    this.subscribedGeneration = -1;
    this.subscribingGeneration = generation;
    const continuousKeys = new Set<string>();
    batch(() => {
      this.subscriptionError.set("");
      for (const [key, interest] of desired.entries()) {
        const entry = this.entry(interest);
        entry.failedGeneration = undefined;
        entry.error.set("");
        const continuousFresh =
          prevInterests.has(key) &&
          entry.phase() === "ready" &&
          entry.appliedGeneration() === prevGeneration &&
          entry.version() >= entry.freshTarget &&
          !!entry.data();
        if (continuousFresh) {
          // Continuously subscribed and up-to-date: carry freshness across the
          // subscription change so shared controls never flash dimmed.
          continuousKeys.add(key);
          entry.appliedGeneration.set(generation);
        } else if (!entry.data() || entry.phase() === "error") {
          entry.phase.set("loading");
        }
        // Revisited cached entries keep phase === "ready" during the subscribe
        // handshake (avoiding UI flicker) while leaving appliedGeneration stale
        // so imperative writes remain fenced until subscribe confirms revision.
      }
    });
    void this.send<StateChanged>("state.subscribe", { generation, domains: [...desired.values()] })
      .then((value) => {
        if (this.disposed || this.generation !== generation) return;
        if (!value || value.generation !== generation || !Array.isArray(value.changes))
          throw new Error("Invalid state subscription receipt");
        this.subscribedGeneration = generation;
        this.applyChanged(value, continuousKeys);
        for (const interest of desired.values()) this.query(this.entry(interest));
      })
      .catch((error) => {
        if (this.disposed || this.generation !== generation) return;
        const message = error instanceof Error ? error.message : String(error);
        batch(() => {
          this.subscriptionError.set(message);
          this.reportError(message);
          for (const interest of desired.values()) {
            const entry = this.entry(interest);
            entry.failedGeneration = generation;
            entry.phase.set("error");
            entry.error.set(message);
          }
        });
        this.checkBarriers();
      });
  }

  private query(entry: Entry): void {
    if (
      entry.flight ||
      this.disposed ||
      !this.connected ||
      this.subscribedGeneration !== this.generation ||
      !this.interests.has(domainKey(entry.interest)) ||
      entry.failedGeneration === this.generation
    )
      return;
    if (
      entry.appliedGeneration() === this.generation &&
      entry.version() >= entry.target &&
      entry.data()
    ) {
      entry.phase.set("ready");
      this.checkBarriers();
      return;
    }
    const generation = this.generation;
    entry.flight = this.send<DomainQueryResult>("state.query", { generation, ...entry.interest })
      .then((value) => {
        if (
          this.disposed ||
          generation !== this.generation ||
          !this.interests.has(domainKey(entry.interest))
        )
          return;
        if (
          !value ||
          value.generation !== generation ||
          value.domain !== entry.interest.domain ||
          value.groupKey !== entry.interest.groupKey ||
          !revision(value.revision) ||
          !revision(value.configRevision)
        )
          throw new Error("Invalid state query identity");
        validateData(value.domain, value.data);
        if (value.revision < entry.target || value.revision < entry.version()) return;
        if (value.domain === "config" && value.configRevision < this.configRevision()) return;
        batch(() => {
          if (value.revision > entry.version() || !entry.data()) this.apply(entry, value);
          if (!deepEqual(entry.observation(), value.observation))
            entry.observation.set(value.observation);
          entry.version.set(value.revision);
          entry.appliedGeneration.set(generation);
          entry.error.set("");
          entry.phase.set("ready");
          this.latestRevision.set(Math.max(this.latestRevision(), value.revision));
          if (value.domain === "config") this.configRevision.set(value.configRevision);
        });
      })
      .catch((error) => {
        if (
          this.disposed ||
          generation !== this.generation ||
          !this.interests.has(domainKey(entry.interest))
        )
          return;
        const message = error instanceof Error ? error.message : String(error);
        batch(() => {
          entry.failedGeneration = generation;
          entry.phase.set("error");
          entry.error.set(message);
          this.reportError(message);
        });
      })
      .finally(() => {
        entry.flight = undefined;
        this.checkBarriers();
        // A change may arrive after .then applies the reply but before this
        // finalizer releases flight. Re-check the target even after success.
        this.query(entry);
      });
  }

  private apply(entry: Entry, value: DomainQueryResult): void {
    if (value.domain !== "profiles") {
      const previous = entry.data();
      if (previous !== undefined && deepEqual(previous, value.data)) return;
      entry.data.set(value.data);
      return;
    }
    const incoming = (value.data as DomainData["profiles"]).profiles;
    const prevUsage = this.profileUsage();
    const usageChanged =
      prevUsage.size !== incoming.length ||
      incoming.some((profile) => {
        const existing = prevUsage.get(profile.id);
        return (
          !existing ||
          existing.uploadBytes !== profile.uploadBytes ||
          existing.downloadBytes !== profile.downloadBytes
        );
      });
    if (usageChanged) {
      this.profileUsage.set(
        new Map(
          incoming.map((profile) => [
            profile.id,
            { uploadBytes: profile.uploadBytes, downloadBytes: profile.downloadBytes },
          ]),
        ),
      );
    }
    if (this.profilesConfigRevision === value.configRevision && entry.data()) return;
    this.profilesConfigRevision = value.configRevision;
    const profiles: Profile[] = incoming.map((profile) => {
      const copy = { ...profile },
        id = profile.id;
      Object.defineProperty(copy, "uploadBytes", {
        enumerable: true,
        get: () => this.profileUsage().get(id)?.uploadBytes || 0,
      });
      Object.defineProperty(copy, "downloadBytes", {
        enumerable: true,
        get: () => this.profileUsage().get(id)?.downloadBytes || 0,
      });
      return copy;
    });
    entry.data.set({ profiles });
  }

  private checkBarriers(): void {
    for (const barrier of [...this.barriers]) {
      let pending = false,
        failure = "";
      for (const required of barrier.required) {
        if (!this.interests.has(domainKey(required))) continue;
        const entry = this.entry(required);
        if (entry.phase() === "error") {
          failure = entry.error() || this.subscriptionError();
          break;
        }
        if (!this.fresh([entry.interest]) || entry.version() < required.revision) pending = true;
      }
      if (failure) this.finishBarrier(barrier, new CommittedStateError(failure));
      else if (!pending) this.finishBarrier(barrier);
    }
  }
  private finishBarrier(barrier: Barrier, error?: Error): void {
    if (!this.barriers.delete(barrier)) return;
    clearTimeout(barrier.timer);
    if (error) barrier.reject(error);
    else barrier.resolve();
  }
  dispose(): void {
    this.disposed = true;
    for (const barrier of [...this.barriers])
      this.finishBarrier(barrier, new CommittedStateError("View disposed"));
    this.leases.clear();
    this.interests.clear();
  }
}

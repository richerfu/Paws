import { batch, cleanup, signal, type Signal, nativeComponent } from "nsh";
import type {
  CoreRequest,
  CoreResult,
  DomainData,
  StateChanged,
  UiState,
  Preferences,
  VpnConfirmation,
} from "./model";
import { CommittedStateError, DomainStore } from "./domain-store";
import { commandDomains } from "./command-data";

const Gateway = nativeComponent<
  { request?: CoreRequest },
  {
    changed: StateChanged;
    result: CoreResult;
    preferences: Preferences;
    received: { id: number };
    operation: { requestId: number; action: VpnAction };
  }
>("paws.core", {
  props: ["request"],
  events: ["changed", "result", "preferences", "received", "operation"],
});

type VpnAction = "start" | "stop" | "restart";
type CommandReceipt<T> = { data: T; stateRefreshError?: string };
type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: number;
  request: CoreRequest;
  action?: VpnAction;
  expired?: boolean;
};
/** One view owns one client. Late replies cannot mutate a disposed view. */
export class CoreClient {
  readonly domains: DomainStore;
  private readonly view = {} as UiState;
  readonly state = (): UiState | undefined =>
    this.domains.has(["config", "status"]) ? this.view : undefined;
  readonly preferences = signal<Preferences>(
    { language: "system", theme: "system", advanced: false, systemLocale: "zh-CN" },
    {
      equals: (previous, next) =>
        previous.language === next.language &&
        previous.theme === next.theme &&
        previous.advanced === next.advanced &&
        previous.systemLocale === next.systemLocale,
    },
  );
  readonly request: Signal<CoreRequest | undefined> = signal<CoreRequest>();
  readonly pending = signal(0);
  readonly pendingRequests = signal<CoreRequest[]>([]);
  isPending(command: string): boolean {
    return this.pendingRequests().some((request) => request.command === command);
  }
  readonly vpnOperation = signal<"start" | "stop" | "restart" | undefined>();
  readonly vpnUnconfirmed = signal<
    { requestId: number; action: "start" | "stop" | "restart" } | undefined
  >();
  readonly notice = signal("");
  readonly warning = signal("");
  readonly error = signal("");
  private sequence = 0;
  private disposed = false;
  private readonly waiting = new Map<number, Pending>();
  private readonly settling = new Set<Pending>();
  private readonly queue: CoreRequest[] = [];
  private sendingId = 0;
  private vpnRequestId = 0;
  private importSequence = 0;
  nextImportIntent(): number {
    return ++this.importSequence;
  }

  constructor(
    private readonly commandTimeoutMs = 180_000,
    private readonly stateTimeoutMs = 15_000,
  ) {
    this.domains = new DomainStore(
      (command, payload) => this.send(command, payload),
      (message) => this.error.set(message),
      stateTimeoutMs,
    );
    const fields: { [D in keyof DomainData]?: (keyof UiState)[] } = {
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
      profiles: ["profiles"],
      proxies: ["proxyGroups"],
      rules: ["rules"],
      providers: ["providers"],
      geodata: ["geodata"],
      traffic: ["traffic", "trafficHistory", "controllerDiagnostics"],
      connections: ["connections"],
      requests: ["requestHistory"],
      logs: ["logs"],
      dns: ["dns"],
      about: ["about"],
    };
    for (const [domain, keys] of Object.entries(fields))
      for (const key of keys!) {
        Object.defineProperty(this.view, key, {
          enumerable: true,
          get: () =>
            (
              this.domains.data(domain as keyof DomainData) as Record<string, unknown> | undefined
            )?.[key],
        });
      }
    Object.defineProperty(this.view, "configRevision", {
      enumerable: true,
      get: () => this.domains.configRevision(),
    });
    Object.defineProperty(this.view, "revision", {
      enumerable: true,
      get: () => this.domains.latestRevision(),
    });
  }

  acceptResult(value: CoreResult): void {
    if (value.id === 0 && !value.ok && !this.disposed) {
      this.error.set(value.error || "Cannot read application state");
      return;
    }
    const pending = this.waiting.get(value.id);
    if (!pending || this.disposed) return;
    this.acceptReceived(value.id);
    clearTimeout(pending.timer);
    this.waiting.delete(value.id);
    if (value.id === this.vpnRequestId) this.vpnOperation.set(undefined);
    if (value.id === this.vpnUnconfirmed()?.requestId) {
      this.vpnUnconfirmed.set(undefined);
      if (pending.expired) {
        if (!value.ok) this.error.set(value.error || "The VPN operation failed");
        else this.error.set("");
      }
    }
    const confirmation = value.data as VpnConfirmation | undefined;
    if (
      pending.request.command === "vpn.confirm" &&
      value.ok &&
      confirmation &&
      confirmation.requestId === this.vpnUnconfirmed()?.requestId
    ) {
      if (confirmation.status === "succeeded" || confirmation.status === "failed")
        this.acceptResult({
          id: confirmation.requestId,
          ok: confirmation.status === "succeeded",
          data: null,
          error: confirmation.error,
        });
      else if (confirmation.status === "unavailable")
        this.error.set(
          confirmation.error ||
            "The original operation receipt is unavailable. Stop and resynchronize.",
        );
    }
    const recovered =
      pending.request.command !== "vpn.recover" ||
      (confirmation?.status === "succeeded" &&
        confirmation.requestId === pending.request.payload.requestId);
    if (value.id === this.vpnRequestId && value.ok && recovered) this.vpnUnconfirmed.set(undefined);
    if (pending.request.command === "vpn.recover" && value.ok && !recovered)
      this.error.set(
        confirmation?.error || "The stop operation is unconfirmed. Confirm it or retry recovery.",
      );
    this.updatePending();
    if (value.ok && value.stateRefreshError) {
      this.domains.retry();
      pending.resolve({
        data: value.data,
        stateRefreshError: new CommittedStateError(value.stateRefreshError).message,
      });
    } else if (value.ok && value.barrier && !pending.request.command.startsWith("state.")) {
      this.settling.add(pending);
      this.updatePending();
      void this.domains
        .waitFor(value.barrier)
        .then(
          () => {
            if (!this.disposed) pending.resolve({ data: value.data });
          },
          (error) => {
            if (!this.disposed)
              pending.resolve({
                data: value.data,
                stateRefreshError: error instanceof Error ? error.message : String(error),
              });
          },
        )
        .finally(() => {
          this.settling.delete(pending);
          this.updatePending();
        });
    } else if (value.ok) pending.resolve({ data: value.data });
    else pending.reject(new Error(value.error || "The operation failed"));
  }

  send<T = unknown>(command: string, payload: Record<string, unknown> = {}): Promise<T> {
    return this.dispatch<T>(command, payload).then((receipt) => {
      if (!this.disposed && receipt.stateRefreshError) this.warning.set(receipt.stateRefreshError);
      return receipt.data;
    });
  }

  private dispatch<T = unknown>(
    command: string,
    payload: Record<string, unknown> = {},
  ): Promise<CommandReceipt<T>> {
    if (this.disposed) return Promise.reject(new Error("View disposed"));
    if (this.domains.observing) {
      if (!this.domains.fresh(commandDomains(command)))
        return Promise.reject(
          new Error("Required state is loading or unavailable. Reload it before making changes."),
        );
      if (
        ["proxies.select", "proxies.unfix"].includes(command) &&
        (!this.domains.fresh(["config"]) ||
          !(
            this.domains.fresh(["proxies"]) ||
            (typeof payload.group === "string" &&
              this.domains.fresh([{ domain: "proxies", groupKey: payload.group }]))
          ))
      )
        return Promise.reject(
          new Error(
            "Proxy group data is loading or unavailable. Reload it before changing selection.",
          ),
        );
      if (
        ["connections.close", "connections.closeAll"].includes(command) &&
        !this.domains.fresh(["connections"]) &&
        !this.domains.fresh(["requests"])
      )
        return Promise.reject(
          new Error(
            "Connection data is loading or unavailable. Reload it before closing connections.",
          ),
        );
    }
    if (
      (command === "profiles.importUrl" || command === "profiles.importContent") &&
      payload.intentId === undefined
    )
      payload = { ...payload, intentId: this.nextImportIntent() };
    const vpnAction =
      command === "vpn.start"
        ? "start"
        : command === "vpn.stop" || command === "vpn.recover"
          ? "stop"
          : command === "vpn.restart"
            ? "restart"
            : undefined;
    if (vpnAction && vpnAction !== "stop" && this.vpnUnconfirmed())
      return Promise.reject(
        new Error(
          "The previous VPN operation is unconfirmed. Confirm it or stop and resynchronize first.",
        ),
      );
    if (
      vpnAction &&
      this.vpnOperation() &&
      (vpnAction !== "stop" || this.vpnOperation() === "stop")
    )
      return Promise.reject(new Error("VPN operation already in progress"));
    const id = ++this.sequence;
    if (vpnAction) {
      this.vpnRequestId = id;
      this.vpnOperation.set(vpnAction);
    }
    const request = { id, command, payload };
    const promise = new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(
        () => {
          const pending = this.waiting.get(id);
          if (!pending) return;
          pending.expired = true;
          if (pending.action && id === this.vpnRequestId) {
            this.vpnOperation.set(undefined);
            this.vpnUnconfirmed.set({ requestId: id, action: pending.action });
          }
          this.updatePending();
          reject(
            new Error(
              pending.action
                ? "The VPN operation is unconfirmed. Confirm the original operation or stop and resynchronize."
                : "The operation timed out without a confirmed final result.",
            ),
          );
        },
        command.startsWith("state.") ? this.stateTimeoutMs : this.commandTimeoutMs,
      );
      this.waiting.set(id, { resolve, reject, timer, request, action: vpnAction });
    });
    this.updatePending();
    this.queue.push(request);
    this.drain();
    return promise as Promise<CommandReceipt<T>>;
  }

  private updatePending(): void {
    const requests = [...this.waiting.values(), ...this.settling]
      .filter((value) => !value.expired && !value.request.command.startsWith("state."))
      .map((value) => value.request);
    this.pendingRequests.set(requests);
    // A decision preview has its own page-local lifetime/loading state. It
    // remains tracked and receives its receipt without blocking real edits.
    this.pending.set(requests.filter((request) => request.command !== "engines.decision").length);
  }

  /** Native fact: this request entered an actual platform VPN effect. */
  acceptOperation(value: { requestId: number; action: VpnAction }): void {
    const pending = this.waiting.get(value.requestId);
    if (this.disposed || !pending || !["start", "stop", "restart"].includes(value.action)) return;
    this.acceptReceived(value.requestId);
    pending.action = value.action;
    if (value.requestId < this.vpnRequestId) return;
    this.vpnRequestId = value.requestId;
    if (pending.expired) {
      this.vpnOperation.set(undefined);
      this.vpnUnconfirmed.set({ requestId: value.requestId, action: value.action });
    } else {
      this.vpnUnconfirmed.set(undefined);
      this.vpnOperation.set(value.action);
    }
  }

  /** Receipt ACK means the host observed the prop; operation completion is independent. */
  acceptReceived(id: number): void {
    if (this.disposed || !Number.isSafeInteger(id) || id !== this.sendingId || id === 0) return;
    this.sendingId = 0;
    this.drain();
  }

  private drain(): void {
    if (this.sendingId !== 0 || this.disposed) return;
    let next = this.queue.shift();
    while (next && !this.waiting.has(next.id)) next = this.queue.shift();
    if (!next) return;
    this.sendingId = next.id;
    this.request.set(next);
  }

  async run<T = unknown>(
    command: string,
    payload: Record<string, unknown> = {},
    success = "",
  ): Promise<T | undefined> {
    batch(() => {
      this.error.set("");
      this.notice.set("");
      this.warning.set("");
    });
    try {
      const receipt = await this.dispatch<T>(command, payload);
      if (!this.disposed && receipt.stateRefreshError) this.warning.set(receipt.stateRefreshError);
      else if (!this.disposed && success) this.notice.set(success);
      return receipt.data;
    } catch (error) {
      if (!this.disposed) {
        const message = error instanceof Error ? error.message : String(error);
        if (error instanceof CommittedStateError) this.warning.set(message);
        else this.error.set(message);
      }
      return undefined;
    }
  }

  dispose(): void {
    this.disposed = true;
    this.domains.dispose();
    for (const pending of this.settling) pending.reject(new CommittedStateError("View disposed"));
    this.settling.clear();
    for (const pending of this.waiting.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error("View disposed"));
    }
    this.waiting.clear();
    this.queue.length = 0;
  }
}

export function CoreGateway(props: { client: CoreClient }) {
  cleanup(() => props.client.dispose());
  props.client.domains.connect();
  return (
    <Gateway
      request={props.client.request()}
      onChanged={(value) => props.client.domains.changed(value)}
      onResult={(value) => props.client.acceptResult(value)}
      onReceived={(value) => props.client.acceptReceived(value.id)}
      onOperation={(value) => props.client.acceptOperation(value)}
      onPreferences={(value) => props.client.preferences.set(value)}
      style={{ position: "absolute", width: 0, height: 0 }}
    />
  );
}
